//! Vector kernels for the bulk of unaligned bit copies.
//!
//! Each kernel processes as many full 16 byte vectors as fit and returns the number of bytes it
//! wrote. The caller handles the remaining bytes. Targets without a kernel process nothing.
//!
//! The kernels only use instructions of the baseline of their target (SSE2 on x86_64, NEON on
//! aarch64), so no runtime feature detection is needed. WebAssembly has no runtime feature
//! detection at all, so its kernel only exists when compiling with `-C target-feature=+simd128`.

#[cfg(any(
    target_arch = "x86_64",
    all(target_arch = "x86", target_feature = "sse2")
))]
pub(super) use x86::shift_left;

#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
pub(super) use aarch64::shift_left;

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub(super) use wasm32::shift_left;

/// No vector kernel for this target.
#[cfg(not(any(
    target_arch = "x86_64",
    all(target_arch = "x86", target_feature = "sse2"),
    all(target_arch = "aarch64", target_feature = "neon"),
    all(target_arch = "wasm32", target_feature = "simd128"),
)))]
#[inline]
pub(super) fn shift_left(_dst: &mut [u8], _src: &[u8], _shift: u32) -> usize {
    0
}

#[cfg(any(
    target_arch = "x86_64",
    all(target_arch = "x86", target_feature = "sse2")
))]
mod x86 {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    /// `dst[i] = src[i] << shift | src[i + 1] >> (8 - shift)` for 16 bytes at a time.
    #[inline]
    pub(in super::super) fn shift_left(dst: &mut [u8], src: &[u8], shift: u32) -> usize {
        assert!(src.len() > dst.len());
        let vectors = dst.len() / 16;
        // SAFETY: SSE2 is part of the x86_64 baseline and enabled on x86 by the `cfg` above.
        // Iteration `i` loads `src[16i..16i + 17]` and stores `dst[16i..16i + 16]`, which are
        // in bounds because `16i + 16 <= dst.len() < src.len()`. The loads and stores are
        // unaligned.
        unsafe {
            // SSE2 cannot shift bytes, so this shifts 16 bit lanes and masks away the bits that
            // crossed into the neighbouring byte
            let left = _mm_cvtsi32_si128(shift as i32);
            let right = _mm_cvtsi32_si128(8 - shift as i32);
            let high = _mm_set1_epi8((0xFF_u8 << shift) as i8);
            let low = _mm_set1_epi8((0xFF_u8 >> (8 - shift)) as i8);
            for i in 0..vectors {
                let current = _mm_loadu_si128(src.as_ptr().add(i * 16).cast());
                let next = _mm_loadu_si128(src.as_ptr().add(i * 16 + 1).cast());
                let out = _mm_or_si128(
                    _mm_and_si128(_mm_sll_epi16(current, left), high),
                    _mm_and_si128(_mm_srl_epi16(next, right), low),
                );
                _mm_storeu_si128(dst.as_mut_ptr().add(i * 16).cast(), out);
            }
        }
        vectors * 16
    }
}

#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
mod aarch64 {
    use core::arch::aarch64::*;

    /// `dst[i] = src[i] << shift | src[i + 1] >> (8 - shift)` for 16 bytes at a time.
    #[inline]
    pub(in super::super) fn shift_left(dst: &mut [u8], src: &[u8], shift: u32) -> usize {
        assert!(src.len() > dst.len());
        let vectors = dst.len() / 16;
        // SAFETY: NEON is enabled according to the `cfg` above. Iteration `i` loads
        // `src[16i..16i + 17]` and stores `dst[16i..16i + 16]`, which are in bounds because
        // `16i + 16 <= dst.len() < src.len()`.
        unsafe {
            // `ushl` shifts left for positive and right for negative shift amounts
            let left = vdupq_n_s8(shift as i8);
            let right = vdupq_n_s8(shift as i8 - 8);
            let pairs = vectors / 2;
            for i in 0..pairs {
                let a = vld1q_u8(src.as_ptr().add(i * 32));
                let b = vld1q_u8(src.as_ptr().add(i * 32 + 16));
                let c = vld1q_u8(src.as_ptr().add(i * 32 + 17));
                let out_a = vorrq_u8(vshlq_u8(a, left), vshlq_u8(vextq_u8::<1>(a, b), right));
                let out_b = vorrq_u8(vshlq_u8(b, left), vshlq_u8(c, right));
                vst1q_u8(dst.as_mut_ptr().add(i * 32), out_a);
                vst1q_u8(dst.as_mut_ptr().add(i * 32 + 16), out_b);
            }
            for i in pairs * 2..vectors {
                let current = vld1q_u8(src.as_ptr().add(i * 16));
                let next = vld1q_u8(src.as_ptr().add(i * 16 + 1));
                let out = vorrq_u8(vshlq_u8(current, left), vshlq_u8(next, right));
                vst1q_u8(dst.as_mut_ptr().add(i * 16), out);
            }
        }
        vectors * 16
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm32 {
    use core::arch::wasm32::*;

    /// `dst[i] = src[i] << shift | src[i + 1] >> (8 - shift)` for 16 bytes at a time.
    #[inline]
    pub(in super::super) fn shift_left(dst: &mut [u8], src: &[u8], shift: u32) -> usize {
        assert!(src.len() > dst.len());
        let vectors = dst.len() / 16;
        for i in 0..vectors {
            // SAFETY: simd128 is enabled according to the `cfg` above. This loads
            // `src[16i..16i + 17]` and stores `dst[16i..16i + 16]`, which are in bounds because
            // `16i + 16 <= dst.len() < src.len()`. WebAssembly allows unaligned accesses.
            unsafe {
                let current = v128_load(src.as_ptr().add(i * 16).cast());
                let next = v128_load(src.as_ptr().add(i * 16 + 1).cast());
                let out = v128_or(i8x16_shl(current, shift), u8x16_shr(next, 8 - shift));
                v128_store(dst.as_mut_ptr().add(i * 16).cast(), out);
            }
        }
        vectors * 16
    }
}
