//! Bit granular copies between byte slices.
//!
//! PER numbers bits MSB-first: bit 0 is the most significant bit of byte 0. All functions in this
//! module leave the bits of the destination outside of the written range untouched.

use super::simd;
use super::BYTE_LEN;
use crate::protocol::per::Error;

const WORD_LEN: usize = u64::BITS as usize;

/// Copies `len` bits from `src`, starting at bit `src_pos`, to `dst`, starting at bit `dst_pos`.
#[inline]
pub(crate) fn copy_bits(
    src: &[u8],
    src_pos: usize,
    dst: &mut [u8],
    dst_pos: usize,
    len: usize,
) -> Result<(), Error> {
    if dst.len() * BYTE_LEN < dst_pos + len {
        return Err(Error::insufficient_space_in_destination_buffer());
    }
    if src.len() * BYTE_LEN < src_pos + len {
        return Err(Error::insufficient_data_in_source_buffer());
    }
    if len <= WORD_LEN {
        write_u64(dst, dst_pos, len, read_u64(src, src_pos, len));
    } else {
        copy_bits_bulk(src, src_pos, dst, dst_pos, len);
    }
    Ok(())
}

#[inline(never)]
fn copy_bits_bulk(src: &[u8], src_pos: usize, dst: &mut [u8], dst_pos: usize, len: usize) {
    // align the destination to a full byte, so that the bulk of the bytes can be written whole
    let head = (BYTE_LEN - dst_pos % BYTE_LEN) % BYTE_LEN;
    write_u64(dst, dst_pos, head, read_u64(src, src_pos, head));

    let src_pos = src_pos + head;
    let dst = &mut dst[(dst_pos + head) / BYTE_LEN..];
    let len = len - head;

    let bytes = len / BYTE_LEN;
    let src_index = src_pos / BYTE_LEN;
    let shift = (src_pos % BYTE_LEN) as u32;
    if shift == 0 {
        dst[..bytes].copy_from_slice(&src[src_index..src_index + bytes]);
    } else {
        // every destination byte combines the tail of one source byte with the head of the next
        shift_left(
            &mut dst[..bytes],
            &src[src_index..=src_index + bytes],
            shift,
        );
    }

    let tail = len % BYTE_LEN;
    let tail_value = read_u64(src, src_pos + bytes * BYTE_LEN, tail);
    write_u64(dst, bytes * BYTE_LEN, tail, tail_value);
}

/// Reads `len <= 64` bits from `src`, starting at bit `pos`, into the least significant bits of
/// the result.
///
/// The caller ensures that `pos + len <= src.len() * 8`.
#[inline]
pub(crate) fn read_u64(src: &[u8], pos: usize, len: usize) -> u64 {
    debug_assert!(len <= WORD_LEN);
    debug_assert!(pos + len <= src.len() * BYTE_LEN);
    if len == 0 {
        return 0;
    }
    let index = pos / BYTE_LEN;
    let shift = pos % BYTE_LEN;
    let mut word = load_be(src, index) << shift;
    if shift + len > WORD_LEN {
        // the range touches a ninth byte, which exists because the range is within `src`
        word |= u64::from(src[index + 8]) >> (BYTE_LEN - shift);
    }
    word >> (WORD_LEN - len)
}

/// Writes the `len <= 64` least significant bits of `value` to `dst`, starting at bit `pos`.
/// Higher bits of `value` are ignored.
///
/// The caller ensures that `pos + len <= dst.len() * 8`.
#[inline]
pub(crate) fn write_u64(dst: &mut [u8], pos: usize, len: usize, value: u64) {
    debug_assert!(len <= WORD_LEN);
    debug_assert!(pos + len <= dst.len() * BYTE_LEN);
    if len == 0 {
        return;
    }
    let shift = pos % BYTE_LEN;
    if shift + len > WORD_LEN {
        // the range touches nine bytes, so write it as two parts that each fit into one word
        let low_pos = pos + len - 32;
        store_be(dst, pos / BYTE_LEN, shift, len - 32, value >> 32);
        store_be(dst, low_pos / BYTE_LEN, low_pos % BYTE_LEN, 32, value);
    } else {
        store_be(dst, pos / BYTE_LEN, shift, len, value);
    }
}

/// Loads the eight bytes starting at `index` as big endian word, padding with zeros past the end
/// of `src`.
#[inline]
fn load_be(src: &[u8], index: usize) -> u64 {
    match src.get(index..index + 8) {
        Some(bytes) => u64::from_be_bytes(bytes.try_into().unwrap()),
        None => src[index..]
            .iter()
            .enumerate()
            .fold(0, |word, (i, byte)| word | u64::from(*byte) << (56 - 8 * i)),
    }
}

/// Stores the `len` least significant bits of `value` at bit `shift` of the big endian word that
/// starts at `index`, for `1 <= len` and `shift + len <= 64`.
#[inline]
fn store_be(dst: &mut [u8], index: usize, shift: usize, len: usize, value: u64) {
    let unused = WORD_LEN - shift - len;
    let mask = (u64::MAX >> (WORD_LEN - len)) << unused;
    let bits = (value << unused) & mask;
    match dst.get_mut(index..index + 8) {
        Some(bytes) => {
            let bytes: &mut [u8; 8] = bytes.try_into().unwrap();
            *bytes = ((u64::from_be_bytes(*bytes) & !mask) | bits).to_be_bytes();
        }
        None => {
            // near the end of `dst`, only touch the bytes that hold bits of the range
            let touched = (shift + len).div_ceil(BYTE_LEN);
            for (i, byte) in dst[index..index + touched].iter_mut().enumerate() {
                let byte_shift = 56 - 8 * i;
                *byte = (*byte & !((mask >> byte_shift) as u8)) | (bits >> byte_shift) as u8;
            }
        }
    }
}

/// Sets `dst[i] = src[i] << shift | src[i + 1] >> (8 - shift)` for every `i`, which shifts the
/// bits of `src` towards the front by `shift` bits.
///
/// Requires `src.len() == dst.len() + 1` and `1 <= shift <= 7`.
#[inline]
fn shift_left(dst: &mut [u8], src: &[u8], shift: u32) {
    debug_assert_eq!(src.len(), dst.len() + 1);
    debug_assert!((1..8).contains(&shift));
    let done = simd::shift_left(dst, src, shift);
    shift_left_swar(&mut dst[done..], &src[done..], shift);
}

/// [`shift_left`] for eight bytes at a time within a `u64`.
#[inline]
#[allow(clippy::chunks_exact_to_as_chunks)] // as_chunks_mut requires Rust 1.88
fn shift_left_swar(dst: &mut [u8], src: &[u8], shift: u32) {
    // Within a little endian word, byte `k` occupies bits `8k..8k + 8`. Shifting the whole word
    // moves bits across byte boundaries, which the masks remove again.
    let high = u64::from_ne_bytes([0xFF << shift; 8]);
    let low = u64::from_ne_bytes([0xFF >> (8 - shift); 8]);

    let words = dst.len() / 8;
    for (i, out) in dst.chunks_exact_mut(8).enumerate() {
        let src = &src[i * 8..i * 8 + 9];
        let current = u64::from_le_bytes(src[..8].try_into().unwrap());
        let next = u64::from_le_bytes(src[1..].try_into().unwrap());
        let word = ((current << shift) & high) | ((next >> (8 - shift)) & low);
        out.copy_from_slice(&word.to_le_bytes());
    }

    let done = words * 8;
    for (out, src) in dst[done..].iter_mut().zip(src[done..].windows(2)) {
        *out = (src[0] << shift) | (src[1] >> (8 - shift));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_bit(bytes: &[u8], pos: usize) -> bool {
        bytes[pos / 8] & (0x80 >> (pos % 8)) != 0
    }

    fn set_bit(bytes: &mut [u8], pos: usize, bit: bool) {
        if bit {
            bytes[pos / 8] |= 0x80 >> (pos % 8);
        } else {
            bytes[pos / 8] &= !(0x80 >> (pos % 8));
        }
    }

    fn copy_bitwise(src: &[u8], src_pos: usize, dst: &mut [u8], dst_pos: usize, len: usize) {
        for i in 0..len {
            set_bit(dst, dst_pos + i, get_bit(src, src_pos + i));
        }
    }

    /// xorshift64*, to not depend on a rand crate
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn bytes(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| self.next() as u8).collect()
        }
    }

    fn assert_copy_matches_bitwise(
        rng: &mut Rng,
        src_pos: usize,
        dst_pos: usize,
        len: usize,
        slack: usize,
    ) {
        let src = rng.bytes((src_pos + len).div_ceil(8) + slack);
        let dst = rng.bytes((dst_pos + len).div_ceil(8) + slack);

        let mut expected = dst.clone();
        copy_bitwise(&src, src_pos, &mut expected, dst_pos, len);

        let mut actual = dst.clone();
        copy_bits(&src, src_pos, &mut actual, dst_pos, len).unwrap();

        assert_eq!(
            expected, actual,
            "src_pos={src_pos}, dst_pos={dst_pos}, len={len}, slack={slack}"
        );
    }

    #[test]
    fn copy_bits_up_to_one_word_matches_bitwise_copy() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for len in 0..=WORD_LEN {
            for src_pos in 0..16 {
                for dst_pos in 0..16 {
                    // without slack, the words at the end of the buffers are incomplete
                    for slack in [0, 8] {
                        assert_copy_matches_bitwise(&mut rng, src_pos, dst_pos, len, slack);
                    }
                }
            }
        }
    }

    #[test]
    fn copy_bits_bulk_matches_bitwise_copy() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        for _ in 0..20_000 {
            let max_len = if rng.below(8) == 0 { 4_000 } else { 300 };
            let len = WORD_LEN + 1 + rng.below(max_len);
            let src_pos = rng.below(64);
            let dst_pos = rng.below(64);
            let slack = rng.below(3);
            assert_copy_matches_bitwise(&mut rng, src_pos, dst_pos, len, slack);
        }
    }

    #[test]
    fn copy_bits_rejects_out_of_bounds_ranges() {
        let src = [0xAA; 4];
        let mut dst = [0x55; 4];
        assert_eq!(
            Err(Error::insufficient_space_in_destination_buffer()),
            copy_bits(&src, 0, &mut dst, 1, 32)
        );
        assert_eq!(
            Err(Error::insufficient_data_in_source_buffer()),
            copy_bits(&src, 1, &mut dst, 0, 32)
        );
        assert_eq!([0x55; 4], dst);
        assert_eq!(Ok(()), copy_bits(&src, 32, &mut dst, 32, 0));
    }

    #[test]
    fn read_u64_reads_msb_first() {
        let src = [
            0b1010_0000,
            0xFF,
            0x00,
            0x12,
            0x34,
            0x56,
            0x78,
            0x9A,
            0xBC,
            0xDE,
        ];
        assert_eq!(0b101, read_u64(&src, 0, 3));
        assert_eq!(0b0_0000_1111, read_u64(&src, 3, 9));
        assert_eq!(0x00_1234_5678_9ABC, read_u64(&src, 16, 56));
        assert_eq!(0xFF00_1234_5678_9ABC, read_u64(&src, 8, 64));
        assert_eq!(0xF800_91A2_B3C4_D5E6, read_u64(&src, 11, 64));
    }

    #[test]
    fn write_u64_ignores_excess_bits_of_the_value() {
        const VALUE: u64 = 0xC3A5_0F96_1E2D_3C4B;
        for len in 0..=WORD_LEN {
            for pos in 0..16 {
                let mut expected = [0x5A; 11];
                for i in 0..len {
                    set_bit(&mut expected, pos + i, (VALUE >> (len - 1 - i)) & 1 != 0);
                }
                let mut actual = [0x5A; 11];
                write_u64(&mut actual, pos, len, VALUE);
                assert_eq!(expected, actual, "pos={pos}, len={len}");
            }
        }
    }

    #[test]
    fn shift_left_matches_bytewise_shift() {
        let mut rng = Rng(0x94D0_49BB_1331_11EB);
        for shift in 1..8 {
            for len in 0..200 {
                // vary the start of the slices, to not only test aligned memory
                let offset = rng.below(16);
                let src = rng.bytes(offset + len + 1);
                let src = &src[offset..];
                let expected = src
                    .windows(2)
                    .map(|w| (w[0] << shift) | (w[1] >> (8 - shift)))
                    .collect::<Vec<_>>();
                let mut actual = rng.bytes(offset + len);
                shift_left(&mut actual[offset..], src, shift);
                assert_eq!(expected, actual[offset..], "shift={shift}, len={len}");
            }
        }
    }
}
