# UPER performance

This document records the optimization of the UPER encoder and decoder for x86_64, arm64 and
WebAssembly. Every change adds a section that backs it with measurements. Experiments that did
not pay off are recorded as well, so that they need not be repeated.

All numbers are medians from `benches/uper.rs`, in nanoseconds per operation. Lower is better.
Speedups are `before / after`.

## Summary

From the original code to change 8 (Store bits byte-wise on wasm32):

| Benchmark | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 → 92.4 ns (3.55×) | 560 → 179 ns (3.13×) | 496 → 196 ns (2.53×) | 480 → 193 ns (2.48×) |
| `datagram/video_fragment/decode` | 220 → 93.2 ns (2.36×) | 548 → 152 ns (3.61×) | 355 → 173 ns (2.05×) | 337 → 174 ns (1.93×) |
| `datagram/transport_feedback_64/encode` | 3,076 → 1,153 ns (2.67×) | 5,756 → 1,901 ns (3.03×) | 4,958 → 1,958 ns (2.53×) | 4,827 → 1,910 ns (2.53×) |
| `datagram/transport_feedback_64/decode` | 2,860 → 1,102 ns (2.59×) | 7,250 → 1,625 ns (4.46×) | 4,494 → 1,876 ns (2.40×) | 4,520 → 1,822 ns (2.48×) |
| `datagram/padding_1200/encode` | 758 → 91.7 ns (8.26×) | 1,751 → 165 ns (10.64×) | 1,118 → 178 ns (6.27×) | 1,117 → 135 ns (8.29×) |
| `datagram/padding_1200/decode` | 716 → 63.4 ns (11.30×) | 1,792 → 136 ns (13.15×) | 1,080 → 151 ns (7.16×) | 1,082 → 106 ns (10.16×) |
| `strings/profile/encode` | 788 → 227 ns (3.47×) | 952 → 472 ns (2.02×) | 773 → 356 ns (2.17×) | 770 → 352 ns (2.18×) |
| `strings/profile/decode` | 253 → 121 ns (2.08×) | 877 → 201 ns (4.36×) | 336 → 230 ns (1.46×) | 334 → 229 ns (1.46×) |
| `bits/write_unaligned_1200` | 647 → 33.8 ns (19.14×) | 1,614 → 80.3 ns (20.09×) | 975 → 93.4 ns (10.44×) | 973 → 51.8 ns (18.79×) |
| `bits/read_unaligned_1200` | 637 → 19.2 ns (33.19×) | 1,609 → 62.4 ns (25.79×) | 969 → 77.7 ns (12.47×) | 967 → 35.0 ns (27.63×) |
| `fields/write_256_mixed_ints` | 1,786 → 420 ns (4.25×) | 3,659 → 1,212 ns (3.02×) | 4,332 → 993 ns (4.36×) | 4,345 → 977 ns (4.45×) |
| `fields/read_256_mixed_ints` | 1,945 → 150 ns (12.99×) | 4,752 → 269 ns (17.64×) | 2,900 → 442 ns (6.56×) | 2,927 → 437 ns (6.70×) |

The changes so far:

| | Change |
|---|---|
| 1 | Copy bits word-at-a-time instead of bit-by-bit |
| 2 | Grow `BitBuffer` with `Vec::resize` |
| 3 | Keep the extension header out of the per-field scope code |
| 4 | Read and write integers as `u64` instead of byte arrays |
| 5 | Shift unaligned bit copies with SSE2, NEON and simd128 |
| 6 | Return `EndOfStream` when reading a bit past the end of a slice (a fix, no speedup) |
| – | Benchmark writing integers into a fixed slice (adds a benchmark, no code change) |
| 7 | Keep zeroed slack after the content of a `BitBuffer` |
| 8 | Store bits byte-wise on wasm32 |
| 9 | Add the PER encoding instruction `[PER: TRAILING]` for `OCTET STRING` (a feature, no speedup) |

The encoded bytes do not change. After every change, a differential test encoded 4000 random
datagrams and 4000 random string sequences (about 11.7 MB of output) and compared the output byte
for byte with that of the original code.

The test suite passes on all four configurations below, except for `proc_macro_coverage_hack`,
which aborts on wasm32 with the original code as well.

## How the numbers were measured

### The benchmark

`benches/uper.rs` uses a minimal timing loop instead of a benchmark framework, so that the same
code also runs as WebAssembly. For each benchmark it calibrates a batch size so that one sample
takes about 20 ms, takes 25 samples and reports the median time per iteration. `BENCH_MS`
changes the total time per benchmark (default 500 ms).

The schema is shaped like the metadata header that a real-time media transport sends in front of
every packet: many constrained integers with odd bit widths (3, 4, 11, 16, 18, 20, 24, 28, 38 and
63 bits), booleans, nested sequences, a `CHOICE`, a `SEQUENCE OF` and `OCTET STRING` payloads at
unaligned bit positions.

| Benchmark | What it measures |
|---|---|
| `datagram/video_fragment` | A sequence of 23 constrained integers and a boolean in nested sequences, inside a `CHOICE` |
| `datagram/transport_feedback_64` | A header followed by a `SEQUENCE OF` 64 sequences of three integers |
| `datagram/padding_{64,1200,16000}` | A header followed by an `OCTET STRING` of 64, 1200 or 16000 bytes at an unaligned position |
| `strings/profile` | Three `UTF8String`s, two of them optional, and a 58 character `IA5String` |
| `bits/{write,read}_unaligned_*` | Copying 16, 1200 or 16000 bytes to or from bit offset 3 of a `BitBuffer` or `Bits` |
| `fields/{write,read}_256_mixed_ints` | 192 constrained integers, 32 booleans and 32 unconstrained integers, written to a `BitBuffer` or read from `Bits` |
| `fields/write_256_mixed_ints_slice` | The same writes into a fixed `(&mut [u8], &mut usize)`, which excludes the cost of growing a `BitBuffer` |
| `datagram/video_fragment_1200/*` | `datagram/video_fragment` with 1200 bytes of media data at its end, in three ways, see [change 9](#9-per-trailing) |

### The machine and the four configurations

All measurements ran on one Apple M5 Pro with macOS 27 (Darwin 27.0.0), Rust 1.99.0 and the
default release profile (`opt-level = 3`, no LTO, 16 codegen units).

| Configuration | How it runs |
|---|---|
| arm64 | Native `aarch64-apple-darwin` |
| x86_64 (Rosetta) | `x86_64-unknown-linux-gnu` in the `rust:1.99-bookworm` container of OrbStack, executed by Rosetta 2 |
| wasm32 | `wasm32-wasip1` in Node.js 22.22.2 (V8) through `node:wasi` |
| wasm32 +simd128 | The same, compiled with `-C target-feature=+simd128` |

Rosetta translates x86_64 instructions to arm64 instructions, so its numbers are not those of an
x86_64 CPU. They show whether a change helps the x86_64 code, but not by how much on real
hardware. Rosetta supports SSE up to SSE4.2 but neither AVX nor AVX2.

The noise differs per configuration. Comparing measurements of code that cannot differ for a
benchmark, for example decoding before and after a change that only touches writing, one run of
the series showed:

| Configuration | Typical difference | Largest difference |
|---|---:|---:|
| arm64 | 1–2% | 7% |
| wasm32 and wasm32 +simd128 | 1–4% | 12–21%, single benchmarks |
| x86_64 (Rosetta) | 6–23% | 41%, whole runs |

On x86_64, whole runs were 20–35% slower than others, probably because macOS scheduled the
OrbStack virtual machine on efficiency cores. Such noise only ever makes a run slower. So the
series ran three times, and the tables report the fastest of the three medians of each
benchmark. Changes in code layout also move unrelated benchmarks by a few percent.

### The series

The tables per change were measured retrospectively, once all changes were done. Every commit was
checked out into its own git worktree, with its own target directory and with the same version of
`benches/uper.rs`, so that every commit runs the same benchmark code, one commit and configuration
after the other. The appendix lists all results.

Change 8 was measured in three more runs after the others, in the same way.

Change 9 was measured in three more runs, with its parent commit as the baseline, whose runtime
code is that of change 8. Both ran the `benches/uper.rs` of the parent, and the benchmarks that
change 9 adds ran from its own `benches/uper.rs`. With its own file, `fields/read_256_mixed_ints`
on arm64 took about 230 ns instead of 150 ns, and so did the parent with only the `append` variant
of the new benchmarks added, which calls no new code: on arm64, that benchmark depends on the code
that the benchmark binary contains elsewhere. Change 9 runs with the `bytes` feature, the parent
without it.

Experiments were measured as A/B comparisons while the changes were made, on the same machine and
on the same day, by switching back and forth between two versions of the code.

### Reproducing

```sh
cargo bench --bench uper [-- <filter>]
```

For WebAssembly, build for `wasm32-wasip1` and run the result in a WASI runtime. This is the
runner that was used for the numbers here:

```js
// wasi-run.mjs: node --no-warnings wasi-run.mjs <file.wasm> [args...]
import { WASI } from 'node:wasi';
import { readFile } from 'node:fs/promises';
import process from 'node:process';

const [, , file, ...args] = process.argv;
const wasi = new WASI({
  version: 'preview1',
  args: [file, ...args],
  env: process.env,
  preopens: { '.': process.cwd(), '/': '/' },
  returnOnExit: true,
});
const module = await WebAssembly.compile(await readFile(file));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
```

```sh
CARGO_TARGET_WASM32_WASIP1_RUNNER="node --no-warnings wasi-run.mjs" \
CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="-C target-feature=+simd128" \
  cargo bench --target wasm32-wasip1 --bench uper
```

To compare commits, check each one out into its own worktree and give each its own
`CARGO_TARGET_DIR`. Cargo identifies the crate in a workspace by its name and version, not by its
path. With a shared target directory, a worktree whose sources are older than the last build is
considered up to date, and the benchmark silently runs the code of another commit.

## Where the time goes in the original code

Every UPER primitive, from a single bit to a 16 KiB `OCTET STRING`, goes through one bit copy
routine, `bit_string_copy_bulked`:

- Ranges of up to 16 bits, and the unaligned head and tail of longer ranges, are copied one bit at
  a time, with index arithmetic and a branch per bit. This covers almost every integer.
- The bytes in between are copied one byte at a time, with a read-modify-write of two destination
  bytes per source byte. Each iteration writes the byte that the next one reads, so LLVM cannot
  vectorize the loop. Compiling the original code with `+simd128` changes nothing:

| Benchmark | wasm32 | wasm32 +simd128 |
|---|---:|---:|
| `bits/read_unaligned_1200` | 982 ns | 969 ns (1.01×) |
| `bits/write_unaligned_16000` | 12,904 ns | 12,828 ns (1.01×) |
| `datagram/video_fragment/encode` | 484 ns | 491 ns (0.99×) |
| `fields/read_256_mixed_ints` | 3,038 ns | 2,950 ns (1.03×) |

- Integers are converted to a big endian `[u8; 8]` with `to_be_bytes` and then copied from that
  array to the buffer with the bit copy, and read the other way around.

The original code copies unaligned bytes at about 1.8 GiB/s on arm64 and spends
7.0 ns on writing and 7.6 ns on reading each field of `fields/*_256_mixed_ints`.

## The changes

### 1. Copy bits word-at-a-time

The bit copy now handles ranges of up to 64 bits, which covers almost every integer, boolean,
length and choice index, with one big endian `u64` load and one masked `u64` store. Where fewer
than eight bytes remain at the end of a buffer, the store falls back to touching only the bytes
in range.

Longer ranges first align the destination to a full byte. If the source is then aligned as well,
the bytes are copied with `copy_from_slice`. Otherwise every destination byte combines two
adjacent source bytes, `src[i] << s | src[i + 1] >> (8 - s)`, which a SWAR loop computes for eight
bytes at a time within a `u64`. At `opt-level = 3`, LLVM vectorizes that loop into two `u64` lanes
per vector register. The disassembly of `copy_bits_bulk` shows `ushl.2d`, `and.16b` and `orr.16b`
on arm64, `movdqu`, `pand` and `por` on x86_64, and `i64x2.shl` with `v128.and` and `v128.or` on
wasm32 with simd128, where LLVM also needs ten `i64x2.extract_lane` and several widening
instructions.

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 → 240 | 1.36× | 1.34× | 1.55× | 1.39× |
| `datagram/video_fragment/decode` | 220 → 90.5 | 2.43× | 3.96× | 1.42× | 1.32× |
| `datagram/transport_feedback_64/encode` | 3,076 → 2,309 | 1.33× | 1.32× | 1.52× | 1.32× |
| `datagram/transport_feedback_64/decode` | 2,860 → 1,300 | 2.20× | 3.39× | 1.62× | 1.61× |
| `datagram/padding_64/encode` | 134 → 99.4 | 1.35× | 1.51× | 1.42× | 1.40× |
| `datagram/padding_64/decode` | 97.3 → 36.5 | 2.67× | 3.86× | 1.74× | 1.81× |
| `datagram/padding_1200/encode` | 758 → 128 | 5.92× | 7.41× | 5.20× | 6.03× |
| `datagram/padding_1200/decode` | 716 → 69.7 | 10.28× | 12.56× | 6.52× | 8.55× |
| `datagram/padding_16000/decode` | 8,415 → 475 | 17.72× | 21.38× | 11.02× | 19.77× |
| `strings/profile/encode` | 788 → 468 | 1.68× | 1.12× | 1.26× | 1.23× |
| `strings/profile/decode` | 253 → 119 | 2.13× | 4.69× | 1.00× | 0.93× |
| `bits/write_unaligned_16` | 13.7 → 12.7 | 1.08× | 1.25× | 1.30× | 1.22× |
| `bits/read_unaligned_16` | 14.5 → 3.7 | 3.92× | 5.15× | 3.98× | 3.96× |
| `bits/write_unaligned_1200` | 647 → 36.2 | 17.88× | 19.82× | 10.13× | 16.81× |
| `bits/read_unaligned_1200` | 637 → 22.1 | 28.83× | 26.16× | 12.99× | 25.92× |
| `bits/write_unaligned_16000` | 8,419 → 430 | 19.60× | 22.35× | 11.85× | 21.91× |
| `bits/read_unaligned_16000` | 8,304 → 298 | 27.83× | 25.93× | 13.41× | 28.30× |
| `fields/write_256_mixed_ints` | 1,786 → 1,537 | 1.16× | 1.20× | 1.83× | 1.71× |
| `fields/read_256_mixed_ints` | 1,945 → 297 | 6.55× | 8.72× | 4.75× | 4.76× |

Reads got much faster, writes much less. The exception among the reads is `strings/profile/decode`
on wasm32, which did not get faster. Writing goes through a `BitBuffer`, which keeps its vector
exactly as long as its content. So nearly every write first grows the vector, and its stores near
the end of the content, where all appending writes happen, never have the eight bytes for a word
store.

Unlike the old byte loop, the new copy keeps the destination bits after the copied range. The old
bit-by-bit path always kept them, so the result now no longer depends on the length of the range.
Encoders never relied on the old behavior, because they write into zeroed bytes.

### 2. Grow `BitBuffer` with `Vec::resize`

`ensure_can_write_additional_bits` runs before every write and grew the buffer through
`extend(repeat(0).take(n))`. `resize` does the same with less overhead.

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 240 → 218 | 1.10× | 1.19× | 1.00× | 1.00× |
| `datagram/transport_feedback_64/encode` | 2,309 → 2,069 | 1.12× | 1.26× | 1.11× | 1.11× |
| `datagram/padding_64/encode` | 99.4 → 97.4 | 1.02× | 1.16× | 1.08× | 1.08× |
| `strings/profile/encode` | 468 → 397 | 1.18× | 1.40× | 1.31× | 1.28× |
| `bits/write_unaligned_16` | 12.7 → 11.9 | 1.07× | 1.21× | 1.06× | 1.06× |
| `fields/write_256_mixed_ints` | 1,537 → 1,340 | 1.15× | 1.44× | 1.08× | 1.03× |

### 3. Keep the extension header out of the per-field scope code

Every field of a sequence first calls `Scope::write_into_field` or `Scope::read_from_field`, which
are inlined into every generated field accessor. Both contained the code that writes or reads the
header of the extension fields of an extensible sequence, which runs at most once per sequence.
This change moves that code into two functions marked `#[inline(never)]`, to keep the per-field
code small when more code gets inlined into it.

On its own, the change made decoding up to 7% slower on arm64, up to 10% on wasm32 and up to 17%
on x86_64 (Rosetta), and encoding on wasm32 up to 1.21× faster:

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 218 → 218 | 1.00× | 0.90× | 1.17× | 1.21× |
| `datagram/video_fragment/decode` | 91.2 → 95.1 | 0.96× | 0.88× | 0.95× | 0.96× |
| `datagram/transport_feedback_64/decode` | 1,298 → 1,264 | 1.03× | 0.83× | 1.04× | 1.00× |
| `datagram/padding_64/decode` | 36.4 → 39.0 | 0.93× | 0.86× | 0.90× | 0.91× |

### 4. Read and write integers as `u64`

After change 2, a profile of writing the integers of `fields/write_256_mixed_ints` into a fixed
slice (macOS `sample`, 3399 samples) showed 89% of the samples in `copy_bits`, which LLVM did not
inline. An integer still took the detour through a byte array: `to_be_bytes` into a stack array,
then the generic copy with arbitrary bit positions within that array.

`BitRead::read_bits_u64` and `BitWrite::write_bits_u64` read and write up to 64 bits directly.
Their default implementations use the existing byte based methods, so other implementations of
the traits keep working, while the slice, `BitBuffer` and `Bits` implementations call the word
load and store directly. The integer primitives, the length determinant and the per-character
codecs of `IA5String`, `NumericString`, `PrintableString` and `VisibleString` use them.

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 218 → 171 | 1.28× | 1.14× | 1.23× | 1.31× |
| `datagram/video_fragment/decode` | 95.1 → 94.2 | 1.01× | 1.04× | 1.53× | 1.53× |
| `datagram/transport_feedback_64/encode` | 2,046 → 1,621 | 1.26× | 1.19× | 1.27× | 1.44× |
| `datagram/transport_feedback_64/decode` | 1,264 → 1,109 | 1.14× | 1.42× | 1.51× | 1.56× |
| `strings/profile/encode` | 409 → 316 | 1.30× | 1.16× | 1.16× | 1.24× |
| `strings/profile/decode` | 130 → 123 | 1.05× | 1.07× | 1.52× | 1.62× |
| `fields/write_256_mixed_ints` | 1,345 → 933 | 1.44× | 1.59× | 1.23× | 1.37× |
| `fields/read_256_mixed_ints` | 306 → 149 | 2.05× | 1.56× | 1.39× | 1.41× |

Change 3 is a prerequisite for this change. Without it, this change made decoding datagrams up to
1.5× slower, see
[Faster integers without moving the extension header](#faster-integers-without-moving-the-extension-header).
Changes 3 and 4 together make everything faster, except for decoding the smallest messages, which
remains up to 5% slower on arm64 and up to 13% slower on x86_64 (Rosetta):

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 218 → 171 | 1.28× | 1.03× | 1.44× | 1.58× |
| `datagram/video_fragment/decode` | 91.2 → 94.2 | 0.97× | 0.92× | 1.45× | 1.46× |
| `datagram/transport_feedback_64/encode` | 2,069 → 1,621 | 1.28× | 1.05× | 1.52× | 1.79× |
| `datagram/transport_feedback_64/decode` | 1,298 → 1,109 | 1.17× | 1.19× | 1.56× | 1.55× |
| `datagram/padding_64/decode` | 36.4 → 38.5 | 0.95× | 0.87× | 1.20× | 1.22× |
| `strings/profile/encode` | 397 → 316 | 1.26× | 0.97× | 1.16× | 1.25× |
| `strings/profile/decode` | 119 → 123 | 0.97× | 0.99× | 1.46× | 1.56× |
| `fields/write_256_mixed_ints` | 1,340 → 933 | 1.44× | 1.34× | 1.22× | 1.38× |
| `fields/read_256_mixed_ints` | 310 → 149 | 2.08× | 1.34× | 1.39× | 1.41× |

### 5. Vector kernels for unaligned copies

This adds 16 byte kernels for the shift loop of change 1:

| Target | Instructions | Notes |
|---|---|---|
| x86 and x86_64 | SSE2: `_mm_sll_epi16` and `_mm_srl_epi16` with masks, because SSE2 cannot shift bytes | SSE2 is part of the x86_64 baseline |
| aarch64 | NEON: `vshlq_u8` with a positive and a negative shift amount | Two vectors per iteration, see [Unrolling](#unrolling-the-vector-kernels) |
| wasm32 | simd128: `i8x16_shl` and `u8x16_shr` | Only with `-C target-feature=+simd128`, as WebAssembly has no runtime feature detection |

The kernels only use baseline features of their targets, so no runtime detection is needed. The
SWAR loop handles the remaining bytes and the targets without a kernel.

With `opt-level = 3`, LLVM already vectorizes the SWAR loop, so the kernels add little:

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/padding_1200/encode` | 107 → 104 | 1.03× | 1.00× | 1.01× | 1.02× |
| `datagram/padding_1200/decode` | 70.7 → 64.8 | 1.09× | 0.97× | 1.00× | 1.02× |
| `datagram/padding_16000/encode` | 520 → 426 | 1.22× | 1.01× | 1.00× | 1.08× |
| `datagram/padding_16000/decode` | 482 → 384 | 1.25× | 1.00× | 1.00× | 1.08× |
| `bits/write_unaligned_1200` | 34.7 → 30.9 | 1.12× | 0.96× | 1.00× | 1.06× |
| `bits/read_unaligned_1200` | 21.9 → 16.4 | 1.34× | 0.99× | 0.99× | 1.12× |
| `bits/write_unaligned_16000` | 430 → 343 | 1.25× | 1.00× | 1.00× | 1.09× |
| `bits/read_unaligned_16000` | 298 → 222 | 1.34× | 1.00× | 1.01× | 1.11× |

On wasm32 without simd128 and on x86_64 the kernels do not exist or do not differ from what LLVM
generates (see [The SSE2 kernel at `opt-level = 3`](#the-sse2-kernel-at-opt-level--3)), so these
columns show only noise. The kernels matter where the loop vectorizer does not run. With
`opt-level = "s"` (and `"z"`), which builds for WebAssembly often use, the SWAR loop stays
scalar, and the kernels make copies of 1200 bytes and more 3.1× to 17× faster on arm64,
3.6× to 19× on wasm32 and 2.2× to 6× on x86_64 (Rosetta):

| Benchmark | arm64 before → after (ns) | x86_64 (Rosetta) before → after (ns) | wasm32 +simd128 before → after (ns) |
|---|---:|---:|---:|
| `datagram/padding_1200/encode` | 385 → 123 (3.13×) | 546 → 249 (2.20×) | 726 → 200 (3.63×) |
| `datagram/padding_1200/decode` | 323 → 60.6 (5.33×) | 453 → 146 (3.10×) | 641 → 110 (5.82×) |
| `datagram/padding_16000/encode` | 3,817 → 445 (8.58×) | 4,975 → 1,113 (4.47×) | 7,739 → 684 (11.31×) |
| `datagram/padding_16000/decode` | 3,677 → 382 (9.63×) | 4,980 → 1,013 (4.92×) | 7,653 → 599 (12.78×) |
| `bits/write_unaligned_1200` | 289 → 38.2 (7.58×) | 394 → 86.5 (4.55×) | 590 → 63.4 (9.31×) |
| `bits/read_unaligned_1200` | 280 → 17.7 (15.84×) | 363 → 61.8 (5.88×) | 569 → 35.7 (15.93×) |
| `bits/write_unaligned_16000` | 3,844 → 355 (10.84×) | 4,901 → 957 (5.12×) | 7,609 → 548 (13.87×) |
| `bits/read_unaligned_16000` | 3,725 → 214 (17.40×) | 4,669 → 817 (5.71×) | 7,474 → 404 (18.50×) |

Because WebAssembly cannot detect features at runtime, the simd128 kernel only exists in builds
with `-C target-feature=+simd128`. All current browsers and runtimes support simd128 (Chrome 91,
Firefox 89, Safari 16.4, Node.js 16.4).

### 6. `EndOfStream` at the end of a slice

`read_bit` for `(&[u8], &mut usize)` compared the position with `>` instead of `>=`, so reading at
the end indexed past the slice and panicked. `Bits` and `BitBuffer` check the position
themselves, so only direct users of the slice reader were affected. The fix only touches the
reader, so the 7% difference of `datagram/video_fragment/encode` on arm64 comes from a changed
code layout:

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 169 → 181 | 0.93× | 1.00× | 1.01× | 1.00× |
| `datagram/video_fragment/decode` | 93.6 → 94.1 | 0.99× | 0.99× | 1.00× | 1.00× |
| `fields/write_256_mixed_ints` | 943 → 938 | 1.00× | 1.00× | 1.01× | 0.98× |
| `fields/read_256_mixed_ints` | 147 → 148 | 0.99× | 1.00× | 1.01× | 1.01× |

### Benchmark: writing into a fixed slice

`fields/write_256_mixed_ints_slice` does the same writes as `fields/write_256_mixed_ints`, but into
a pre-sized `(&mut [u8], &mut usize)`, which excludes the cost of growing a `BitBuffer`. Measured
on arm64 for every commit so far:

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `fields/write_256_mixed_ints` | 1,786 | 1,537 | 1,340 | 1,345 | 933 | 943 | 938 | 428 | 420 |
| `fields/write_256_mixed_ints_slice` | 1,106 | 789 | 792 | 795 | 685 | 683 | 684 | 680 | 682 |

After change 6, writing into the `BitBuffer` takes 1.37× as long as writing into the slice. The
appendix lists the other configurations.

### 7. Zeroed slack after the content of a `BitBuffer`

Before this change, `fields/write_256_mixed_ints` took 938 ns on arm64, but writing the
same fields into a fixed slice took 684 ns. The difference was the `BitBuffer`: it
grew its vector on almost every write, and its word stores at the end of the content had to fall
back to byte-wise stores.

The vector now holds at least eight zeroed bytes after the content and grows ahead in chunks
(to at least twice its length, at least 64 bytes). A separate length tracks the content.
`content`, `byte_len`, `Debug` and the conversion into a `Vec` report only the content, as before,
and reads stop at its end. Growing never reallocates a buffer whose capacity fits the content.
The writes into its last bytes then store byte-wise as before. `clear` zeroes the content instead
of truncating the vector, so that a reused buffer does not grow again.

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 181 → 91.9 | 1.97× | 1.81× | 1.07× | 1.06× |
| `datagram/transport_feedback_64/encode` | 1,644 → 1,209 | 1.36× | 1.69× | 0.96× | 0.93× |
| `datagram/padding_64/encode` | 82.1 → 71.0 | 1.16× | 1.33× | 0.97× | 0.96× |
| `datagram/padding_1200/encode` | 104 → 92.0 | 1.13× | 1.29× | 0.98× | 0.97× |
| `strings/profile/encode` | 314 → 240 | 1.31× | 1.32× | 0.81× | 0.78× |
| `bits/write_unaligned_16` | 11.0 → 10.9 | 1.01× | 0.96× | 0.91× | 0.94× |
| `bits/write_unaligned_1200` | 30.8 → 34.6 | 0.89× | 1.00× | 0.98× | 0.97× |
| `bits/write_unaligned_16000` | 342 → 348 | 0.98× | 1.00× | 1.01× | 1.00× |
| `fields/write_256_mixed_ints` | 938 → 428 | 2.19× | 1.30× | 1.46× | 1.50× |
| `fields/write_256_mixed_ints_slice` | 684 → 680 | 1.01× | 0.99× | 1.00× | 1.00× |
| `datagram/video_fragment/decode` | 94.1 → 93.7 | 1.00× | 1.00× | 1.00× | 1.00× |
| `fields/read_256_mixed_ints` | 148 → 146 | 1.01× | 1.05× | 0.99× | 1.00× |

`bits/write_unaligned_1200` got slower on arm64. That benchmark clears and refills the same
buffer, and the slowdown is a constant of about 5 ns per iteration, which does not depend on the
buffer having spare capacity. See [`BitBuffer` slack variants](#bitbuffer-slack-variants).

On wasm32, the change made `strings/profile/encode` about 20% slower and other encoders up to 9%
slower, because appending writes now use the word store, which is expensive there.

### 8. Byte-wise stores on wasm32

Change 7 made encoding slower on wasm32. WebAssembly has no byte swap instruction, so LLVM expands
each 64-bit byte swap of the big endian word into about twenty instructions, and a word store
needs two of them. On wasm32, the store now always touches the bytes of the range one by one,
which is what it did before change 7 for all appending writes. Native targets keep the word store,
and their code does not change, so the arm64 and x86_64 columns show only noise.

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 91.9 → 92.4 | 0.99× | 1.01× | 1.05× | 1.07× |
| `datagram/transport_feedback_64/encode` | 1,209 → 1,153 | 1.05× | 1.00× | 1.03× | 1.04× |
| `datagram/padding_64/encode` | 71.0 → 71.1 | 1.00× | 1.00× | 1.04× | 1.05× |
| `datagram/padding_1200/encode` | 92.0 → 91.7 | 1.00× | 1.00× | 1.02× | 1.04× |
| `strings/profile/encode` | 240 → 227 | 1.06× | 1.00× | 1.37× | 1.40× |
| `bits/write_unaligned_16` | 10.9 → 11.0 | 0.99× | 1.00× | 1.19× | 1.14× |
| `fields/write_256_mixed_ints` | 428 → 420 | 1.02× | 1.00× | 1.25× | 1.28× |
| `fields/write_256_mixed_ints_slice` | 680 → 682 | 1.00× | 1.00× | 2.20× | 2.19× |
| `datagram/video_fragment/decode` | 93.7 → 93.2 | 1.01× | 0.99× | 1.00× | 1.00× |
| `strings/profile/decode` | 122 → 121 | 1.00× | 0.99× | 1.00× | 1.01× |

See [Byte-wise stores and reads on wasm32](#byte-wise-stores-and-reads-on-wasm32) for the variants
that were measured.

### 9. `[PER: TRAILING]`

A datagram that ends with a payload of octets, such as a fragment of a media frame, needs no
length for it: the payload runs to the end of the datagram. UPER cannot express that, so an
application encodes the other fields, appends the payload after the padding bits and splits it off
again after decoding. Change 9 adds the PER encoding instruction `[PER: TRAILING]` for this layout.
The writer fills the bits up to the next octet boundary with zeros and copies the octets after
them, and the reader takes all octets from that boundary to the end. With the new `bytes` feature,
an `OCTET STRING` can be held in a `bytes::Bytes`, which `SharedBits` reads from a `Bytes` without
copying the octets.

`datagram/video_fragment_1200` encodes and decodes the `CHOICE` of `datagram/video_fragment` with
1200 bytes of media data at its end, in three ways that produce the same bytes:

| Variant | Encode | Decode |
|---|---|---|
| `append` | The fields into a `UperWriter`, then the data appended to its `Vec<u8>` | The fields from `Bits`, then the data sliced off the input `Bytes` |
| `trailing_vec` | `[PER: TRAILING] OCTET STRING` from a `Vec<u8>` | The same into a `Vec<u8>`, from `Bits` |
| `trailing_bytes` | `[PER: TRAILING] OCTET STRING` from a `Bytes` | The same into a `Bytes`, from `SharedBits` |

| Benchmark | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment_1200/append/encode` | 133 | 214 | 230 | 228 |
| `datagram/video_fragment_1200/trailing_vec/encode` | 126 | 228 | 244 | 240 |
| `datagram/video_fragment_1200/trailing_bytes/encode` | 126 | 230 | 244 | 240 |
| `datagram/video_fragment_1200/append/decode` | 86.6 | 135 | 181 | 177 |
| `datagram/video_fragment_1200/trailing_vec/decode` | 144 | 208 | 235 | 232 |
| `datagram/video_fragment_1200/trailing_bytes/decode` | 82.1 | 140 | 194 | 191 |

On arm64, `[PER: TRAILING]` with `Bytes` encodes and decodes as fast as appending and slicing. On
x86_64 and wasm32, it encodes 12–16 ns (5–7%) slower, which on x86_64 is within the typical noise.
The writer gets enough capacity for the whole datagram, but `BitBuffer` fills the space it grows
into with zeros before the data is copied there, which appending avoids. That zero fill is the
likely cause, but it was not measured separately. On wasm32, decoding into `Bytes` is 13–14 ns
(7–8%) slower than slicing, for a reason not investigated yet. Decoding into a `Vec<u8>` copies the
data and takes 40–70 ns longer than into a `Bytes`.

Encodings without `[PER: TRAILING]` do not change: a differential test encoded and decoded 4000
random datagrams and 4000 random string sequences of the benchmark schema (3.6 MB of output) with
the parent commit and with change 9, and the output was the same byte for byte.

The other benchmarks, with the same `benches/uper.rs` for both commits:

| Benchmark | arm64 before → after (ns) | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 91.9 → 94.8 | 0.97× | 1.02× | 1.00× | 0.99× |
| `datagram/video_fragment/decode` | 93.1 → 95.9 | 0.97× | 0.97× | 1.00× | 0.99× |
| `datagram/transport_feedback_64/encode` | 1,166 → 1,180 | 0.99× | 1.02× | 0.97× | 0.99× |
| `datagram/transport_feedback_64/decode` | 1,104 → 1,105 | 1.00× | 0.99× | 1.04× | 1.00× |
| `datagram/padding_1200/encode` | 92.2 → 92.6 | 1.00× | 1.03× | 1.00× | 0.98× |
| `datagram/padding_1200/decode` | 63.5 → 65.8 | 0.97× | 1.02× | 1.01× | 1.00× |
| `strings/profile/encode` | 228 → 234 | 0.97× | 1.02× | 0.99× | 1.01× |
| `strings/profile/decode` | 119 → 119 | 1.00× | 1.01× | 1.01× | 1.00× |
| `bits/read_unaligned_1200` | 16.8 → 19.1 | 0.88× | 1.00× | 1.03× | 1.00× |
| `fields/write_256_mixed_ints` | 420 → 440 | 0.95× | 1.02× | 0.99× | 1.02× |
| `fields/read_256_mixed_ints` | 141 → 141 | 1.00× | 1.01× | 0.99× | 1.00× |

Every value that a `UperWriter` writes now first checks whether an `OCTET STRING` with
`[PER: TRAILING]` came before it. Apart from that, the code of these benchmarks does not change.
`bits/read_unaligned_1200` and `fields/write_256_mixed_ints` do not run any changed code, but were
slower on arm64 in all three runs, so the 0.88× and 0.95× come from the code layout. The appendix
lists all benchmarks.

## Experiments that did not pay off

### Word-aligned store windows

After change 2, writes were much slower than reads. The hypothesis: each word store
read-modify-writes the eight bytes from the written byte on, so the next write loads eight bytes
that only partially overlap the previous store. CPUs cannot forward such a store to the load, so
the load waits until the store has been committed. Storing into words at multiples of eight bytes
instead would make consecutive writes into the same word access exactly the same memory.

Measured on arm64 after change 2, with the same writes into a fixed slice as an additional
benchmark:

| Stores | `fields/write_256_mixed_ints` | Into a fixed slice | `fields/read_256_mixed_ints` |
|---|---:|---:|---:|
| Word at the written byte (as committed) | 1,353 ns | 787 ns | 305 ns |
| Word at a multiple of eight bytes | 1,426 ns | 775 ns | 214 ns |
| Byte-wise only | 1,362 ns | 985 ns | 322 ns |

The aligned words did not make the slice writer faster, so the partial overlap was not what made
writes slow. The read benchmark moved although no read code changed, which shows how much code
layout alone can move these numbers. The byte-wise variant confirms that the word store is worth
it: it made the slice writer 25% slower. The `BitBuffer` writer did not change, because it never
has the eight bytes for a word store. The profile then found the real cost, which change 4
removed.

### Faster integers without moving the extension header

The first version of change 4 did not include change 3. It made writing faster, but decoding
datagrams slower than before:

| Benchmark | after change 2 | u64 integers only | u64 integers and change 3 |
|---|---:|---:|---:|
| `datagram/video_fragment/decode` | 90.8 ns | 136 ns (0.67×) | 92.7 ns (0.98×) |
| `datagram/transport_feedback_64/decode` | 1,289 ns | 1,548 ns (0.83×) | 1,094 ns (1.18×) |
| `datagram/padding_64/decode` | 36.1 ns | 51.1 ns (0.71×) | 37.3 ns (0.97×) |
| `datagram/padding_1200/decode` | 68.9 ns | 82.8 ns (0.83×) | 69.5 ns (0.99×) |
| `fields/read_256_mixed_ints` | 310 ns | 142 ns (2.19×) | 147 ns (2.11×) |
| `datagram/video_fragment/encode` | 219 ns | 177 ns (1.24×) | 170 ns (1.29×) |

A profile of `datagram/video_fragment/decode` (3397 samples) showed 61% of the samples in
`UperReader::read_bit_field_entry`. It had grown to about 5 KB of code because the extension
header code inlined the whole integer read path, and its prologue saved and restored all
callee-saved registers, including `d8` to `d15`, on every call, even for a field that is neither
optional nor extensible. Change 3 moved the extension header code out, and in this comparison,
decoding returned to about its previous speed. The combined table under change 4 shows the
remaining difference.

### Unrolling the vector kernels

The first NEON kernel processed one vector per iteration. Processing two vectors per iteration, and
taking the second source vector of the first half from `vextq_u8` instead of an unaligned load,
was measurably faster, so the committed kernel does that:

| Benchmark | SWAR loop only | NEON, 1 vector | NEON, 2 vectors |
|---|---:|---:|---:|
| `bits/write_unaligned_1200` | 35.7 ns | 31.6 ns (1.13×) | 31.1 ns (1.15×) |
| `bits/read_unaligned_1200` | 21.7 ns | 21.4 ns (1.01×) | 16.9 ns (1.28×) |
| `bits/write_unaligned_16000` | 426 ns | 366 ns (1.16×) | 359 ns (1.19×) |
| `bits/read_unaligned_16000` | 302 ns | 246 ns (1.23×) | 216 ns (1.40×) |
| `datagram/padding_16000/encode` | 515 ns | 480 ns (1.07×) | 441 ns (1.17×) |
| `datagram/padding_16000/decode` | 498 ns | 456 ns (1.09×) | 435 ns (1.14×) |

The same unrolling for simd128, with four unaligned loads per iteration, gained at most 6%
over one vector per iteration, which is close to the noise. The committed simd128 kernel keeps
one vector per iteration:

| Benchmark | SWAR loop only | simd128, 1 vector | simd128, 2 vectors |
|---|---:|---:|---:|
| `bits/write_unaligned_1200` | 57.5 ns | 52.5 ns (1.10×) | 51.3 ns (1.12×) |
| `bits/read_unaligned_1200` | 40.8 ns | 34.9 ns (1.17×) | 33.1 ns (1.23×) |
| `bits/write_unaligned_16000` | 623 ns | 541 ns (1.15×) | 521 ns (1.20×) |
| `bits/read_unaligned_16000` | 485 ns | 405 ns (1.20×) | 387 ns (1.25×) |
| `datagram/padding_16000/encode` | 700 ns | 638 ns (1.10×) | 621 ns (1.13×) |
| `datagram/padding_16000/decode` | 689 ns | 608 ns (1.13×) | 573 ns (1.20×) |

The SSE2 kernel processes one vector per iteration. Rosetta translates it to arm64 code, so
measurements there could not tell whether unrolling helps an x86_64 CPU.

### The SSE2 kernel at `opt-level = 3`

At `opt-level = 3`, the SSE2 kernel is not faster than the SWAR loop:

| Benchmark | SWAR loop only | SSE2 kernel |
|---|---:|---:|
| `bits/write_unaligned_1200` | 78.3 ns | 81.9 ns (0.96×) |
| `bits/read_unaligned_1200` | 63.4 ns | 62.4 ns (1.02×) |
| `bits/write_unaligned_16000` | 972 ns | 948 ns (1.02×) |
| `bits/read_unaligned_16000` | 851 ns | 818 ns (1.04×) |
| `datagram/padding_16000/encode` | 1,063 ns | 1,090 ns (0.98×) |
| `datagram/padding_16000/decode` | 1,019 ns | 1,006 ns (1.01×) |

The reason: LLVM already vectorizes the SWAR loop with SSE2, which is part of the x86_64 baseline.
The disassembly of `copy_bits_bulk` without the kernel contains `movdqu`, `pand` and `por`, so it
processes two `u64` words per SSE register. The kernel stays because it keeps the copy vectorized
where the loop vectorizer does not run (see change 5).

### AVX2

There is no AVX2 kernel. Rosetta does not support AVX, so an AVX2 kernel could neither be tested
nor measured on the available machine. AVX2 would need runtime detection or a build for x86-64-v3.
An AVX2 kernel would process 32 instead of 16 bytes per iteration, which can only matter for
copies of several KiB, as a 1200 byte copy already takes only about 75 iterations of the SSE2
kernel.

### Word-aligned store windows after change 7

After change 7, integers no longer go through `copy_bits`, and `BitBuffer` writes use word stores,
so the experiment with word-aligned stores was repeated, alternating between the two variants:

| Benchmark | as committed (run 1) | as committed (run 2) | aligned (run 1) | aligned (run 2) |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 92.4 ns | 91.9 ns (1.01×) | 111 ns (0.83×) | 110 ns (0.84×) |
| `datagram/transport_feedback_64/encode` | 1,166 ns | 1,166 ns (1.00×) | 1,318 ns (0.89×) | 1,318 ns (0.88×) |
| `datagram/padding_64/encode` | 71.8 ns | 71.8 ns (1.00×) | 74.4 ns (0.97×) | 73.9 ns (0.97×) |
| `datagram/padding_1200/encode` | 92.4 ns | 93.0 ns (0.99×) | 97.0 ns (0.95×) | 96.3 ns (0.96×) |
| `datagram/padding_16000/encode` | 428 ns | 425 ns (1.01×) | 432 ns (0.99×) | 426 ns (1.00×) |
| `strings/profile/encode` | 233 ns | 234 ns (1.00×) | 248 ns (0.94×) | 249 ns (0.94×) |
| `fields/write_256_mixed_ints` | 422 ns | 421 ns (1.00×) | 528 ns (0.80×) | 539 ns (0.78×) |
| `fields/write_256_mixed_ints_slice` | 678 ns | 680 ns (1.00×) | 492 ns (1.38×) | 493 ns (1.38×) |
| `fields/read_256_mixed_ints` | 146 ns | 145 ns (1.01×) | 142 ns (1.02×) | 150 ns (0.97×) |

Now the aligned words make the fixed slice writer 1.38× faster, so stores that cannot be forwarded
do cost time there. But they make `BitBuffer` writes, which `UperWriter` uses, about 20% slower,
and encoding datagrams up to 17% slower. Aligned words split every field that crosses a multiple
of 64 bits into two stores, while the committed code only splits fields of 58 bits and more.
Why the `BitBuffer` writer loses more than it gains was not investigated further.

### `BitBuffer` slack variants

The first version of change 7 had two problems:

- `clear` truncated the vector, so a reused buffer grew and zeroed its slack again after every
  `clear`.
- The slack was required, so a buffer whose capacity exactly fit its content was reallocated to
  twice its size.

| Benchmark | after change 6 | first version | as committed (run 1) | as committed (run 2) |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 174 ns | 94.0 ns (1.85×) | 96.3 ns (1.81×) | 92.1 ns (1.89×) |
| `datagram/transport_feedback_64/encode` | 1,602 ns | 1,164 ns (1.38×) | 1,272 ns (1.26×) | 1,182 ns (1.35×) |
| `strings/profile/encode` | 322 ns | 249 ns (1.29×) | 232 ns (1.38×) | 241 ns (1.34×) |
| `bits/write_unaligned_16` | 11.2 ns | 13.2 ns (0.85×) | 11.0 ns (1.02×) | 10.9 ns (1.03×) |
| `bits/write_unaligned_1200` | 30.9 ns | 33.6 ns (0.92×) | 34.7 ns (0.89×) | 35.0 ns (0.88×) |
| `bits/read_unaligned_1200` | 16.0 ns | 19.1 ns (0.84×) | 18.2 ns (0.88×) | 16.8 ns (0.95×) |
| `fields/write_256_mixed_ints` | 958 ns | 414 ns (2.31×) | 428 ns (2.24×) | 423 ns (2.26×) |

Two follow-up checks:

- `bits/read_unaligned_1200` reads through `Bits`, whose code did not change. Its apparent
  slowdown came from where the benchmark's source `BitBuffer` was allocated. Reading from an
  independent copy of the bytes shows no difference:

| Benchmark | after change 6 | slack, `clear` zeroing |
|---|---:|---:|
| `bits/read_unaligned_16` | 3.6 ns | 3.6 ns (1.00×) |
| `bits/read_unaligned_1200` | 16.9 ns | 16.6 ns (1.02×) |
| `bits/read_unaligned_16000` | 219 ns | 215 ns (1.02×) |

- The remaining slowdown of `bits/write_unaligned_1200` stays when the buffer has 64 bytes of
  spare capacity, so the byte-wise stores at the end of an exactly sized buffer do not cause it:

| Benchmark | after change 6 (run 1) | after change 6 (run 2) | as committed (run 1) | as committed (run 2) |
|---|---:|---:|---:|---:|
| `bits/write_unaligned_16` | 11.1 ns | 11.2 ns (0.99×) | 9.7 ns (1.14×) | 9.8 ns (1.13×) |
| `bits/write_unaligned_1200` | 30.5 ns | 30.5 ns (1.00×) | 35.7 ns (0.85×) | 35.8 ns (0.85×) |
| `bits/write_unaligned_16000` | 351 ns | 348 ns (1.01×) | 344 ns (1.02×) | 352 ns (1.00×) |

  It is a constant of about 5 ns per `clear` and refill of the same buffer, which `UperWriter`
  never does. A likely but unverified cause is the first word store after the `memset` of `clear`,
  which may not be forwarded from the `memset` stores.

### Byte-wise stores and reads on wasm32

Change 8 stores byte-wise on wasm32 for ranges of any size. Storing byte-wise only for ranges of up
to two bytes, and with the word store otherwise, was slower. Each column is the fastest of three
interleaved runs, after change 7; the decoders, which do not store, serve as a control:

| Benchmark | word store | byte-wise up to 2 bytes | always byte-wise |
|---|---:|---:|---:|
| `datagram/video_fragment/encode` | 206 ns | 200 ns (1.03×) | 193 ns (1.06×) |
| `datagram/transport_feedback_64/encode` | 1,948 ns | 1,954 ns (1.00×) | 1,915 ns (1.02×) |
| `datagram/padding_1200/encode` | 140 ns | 137 ns (1.02×) | 137 ns (1.03×) |
| `strings/profile/encode` | 497 ns | 370 ns (1.34×) | 355 ns (1.40×) |
| `bits/write_unaligned_16` | 19.9 ns | 19.2 ns (1.04×) | 18.7 ns (1.06×) |
| `fields/write_256_mixed_ints` | 1,255 ns | 1,111 ns (1.13×) | 1,024 ns (1.23×) |
| `fields/write_256_mixed_ints_slice` | 1,536 ns | 1,219 ns (1.26×) | 704 ns (2.18×) |
| `datagram/video_fragment/decode` | 176 ns | 174 ns (1.01×) | 174 ns (1.01×) |
| `strings/profile/decode` | 231 ns | 231 ns (1.00×) | 232 ns (0.99×) |

That is wasm32 with simd128. Without simd128, the speedups were the same within three percentage
points:

| Benchmark | word store | byte-wise up to 2 bytes | always byte-wise |
|---|---:|---:|---:|
| `datagram/video_fragment/encode` | 208 ns | 204 ns (1.02×) | 196 ns (1.06×) |
| `datagram/transport_feedback_64/encode` | 2,033 ns | 1,997 ns (1.02×) | 1,937 ns (1.05×) |
| `datagram/padding_1200/encode` | 185 ns | 182 ns (1.02×) | 179 ns (1.03×) |
| `strings/profile/encode` | 491 ns | 374 ns (1.31×) | 355 ns (1.38×) |
| `bits/write_unaligned_16` | 19.3 ns | 17.6 ns (1.10×) | 18.0 ns (1.07×) |
| `fields/write_256_mixed_ints` | 1,246 ns | 1,124 ns (1.11×) | 993 ns (1.25×) |
| `fields/write_256_mixed_ints_slice` | 1,536 ns | 1,219 ns (1.26×) | 700 ns (2.19×) |
| `datagram/video_fragment/decode` | 174 ns | 174 ns (1.00×) | 174 ns (1.00×) |
| `strings/profile/decode` | 230 ns | 231 ns (0.99×) | 232 ns (0.99×) |

Reads load a big endian word as well, but only one per read, and need no store. On top of change
8, assembling the bytes of every read one by one on wasm32 made some decoders faster and others
slower, so reads keep the word load:

| Benchmark | byte-wise stores | and byte-wise reads |
|---|---:|---:|
| `datagram/video_fragment/decode` | 175 ns | 169 ns (1.04×) |
| `datagram/transport_feedback_64/decode` | 1,852 ns | 1,933 ns (0.96×) |
| `datagram/padding_64/decode` | 72.9 ns | 71.7 ns (1.02×) |
| `strings/profile/decode` | 232 ns | 190 ns (1.22×) |
| `fields/read_256_mixed_ints` | 434 ns | 407 ns (1.07×) |
| `bits/read_unaligned_16` | 8.4 ns | 8.8 ns (0.95×) |
| `datagram/video_fragment/encode` | 194 ns | 193 ns (1.00×) |

`datagram/transport_feedback_64/decode` reads many 63-bit integers, which span eight or nine
bytes. Reading only ranges of up to four bytes byte-wise, and the others with the word load, was
slower than both variants, in all decoders:

| Benchmark | byte-wise stores | and byte-wise reads up to 4 bytes |
|---|---:|---:|
| `datagram/video_fragment/decode` | 174 ns | 210 ns (0.83×) |
| `datagram/transport_feedback_64/decode` | 1,830 ns | 2,177 ns (0.84×) |
| `datagram/padding_64/decode` | 71.8 ns | 85.8 ns (0.84×) |
| `strings/profile/decode` | 230 ns | 289 ns (0.80×) |
| `bits/read_unaligned_16` | 8.7 ns | 8.7 ns (1.00×) |

Without simd128, both read experiments showed the same speedups within five percentage points.

## Findings not addressed

- `UperReader::read_whole_sub_slice` limits the readable length through
  `core::mem::replace(&mut self.bits.len(), write_position)`, which replaces a temporary. The
  length is never limited.
- `Bits::read_bits*` check the position against the underlying slice, not against the length set
  through `ScopedBitRead::set_len`.
- `Error::insufficient_data_in_source_buffer` and `Error::insufficient_space_in_destination_buffer`
  capture a backtrace, which makes decoders that probe for the end of the input slow on that
  path.

- At `opt-level = "z"`, writing 16000 bytes into a `BitBuffer` took about 4 µs on arm64 while
  reading them took 0.2 µs (measured after change 5).

- Writing is still slower than reading. A writer that collects bits in a register and stores
  whole words would avoid the read-modify-write per field, but `BitBuffer` exposes its bytes
  through `content(&self)`, which would then have to flush first.
- Reads on wasm32 still load a big endian word, which costs an expanded byte swap. Neither reading
  byte-wise nor a size threshold helped (see
  [Byte-wise stores and reads on wasm32](#byte-wise-stores-and-reads-on-wasm32)), but a decoder
  that keeps the stream in little endian words could avoid the swaps.
- `BitBuffer` zeroes the space it grows into, also when an aligned copy of many bytes will
  overwrite it right away. Encoding with `[PER: TRAILING]` is probably slower than appending on
  x86_64 and wasm32 because of that (see [change 9](#9-per-trailing)).
- On wasm32, decoding from `SharedBits` is 7–8% slower than decoding from `Bits` and slicing the
  data off.
- On arm64, `fields/read_256_mixed_ints` takes 150 ns or 230–340 ns, depending on unrelated code in
  the benchmark binary (see [The series](#the-series)).

## Appendix: all results

The medians of the series in ns, for every commit so far, and the speedup from the
original code to the last change.

### arm64

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 | 240 | 218 | 218 | 171 | 169 | 181 | 91.9 | 92.4 | 3.55× |
| `datagram/video_fragment/decode` | 220 | 90.5 | 91.2 | 95.1 | 94.2 | 93.6 | 94.1 | 93.7 | 93.2 | 2.36× |
| `datagram/transport_feedback_64/encode` | 3,076 | 2,309 | 2,069 | 2,046 | 1,621 | 1,619 | 1,644 | 1,209 | 1,153 | 2.67× |
| `datagram/transport_feedback_64/decode` | 2,860 | 1,300 | 1,298 | 1,264 | 1,109 | 1,103 | 1,102 | 1,107 | 1,102 | 2.59× |
| `datagram/padding_64/encode` | 134 | 99.4 | 97.4 | 99.4 | 85.7 | 82.9 | 82.1 | 71.0 | 71.1 | 1.89× |
| `datagram/padding_64/decode` | 97.3 | 36.5 | 36.4 | 39.0 | 38.5 | 37.1 | 37.1 | 36.3 | 36.1 | 2.70× |
| `datagram/padding_1200/encode` | 758 | 128 | 122 | 123 | 107 | 104 | 104 | 92.0 | 91.7 | 8.26× |
| `datagram/padding_1200/decode` | 716 | 69.7 | 69.4 | 71.5 | 70.7 | 64.8 | 65.0 | 63.6 | 63.4 | 11.30× |
| `datagram/padding_16000/encode` | 8,447 | 534 | 521 | 521 | 520 | 426 | 433 | 412 | 420 | 20.10× |
| `datagram/padding_16000/decode` | 8,415 | 475 | 478 | 473 | 482 | 384 | 397 | 394 | 400 | 21.05× |
| `strings/profile/encode` | 788 | 468 | 397 | 409 | 316 | 318 | 314 | 240 | 227 | 3.47× |
| `strings/profile/decode` | 253 | 119 | 119 | 130 | 123 | 120 | 120 | 122 | 121 | 2.08× |
| `bits/write_unaligned_16` | 13.7 | 12.7 | 11.9 | 13.3 | 11.1 | 11.3 | 11.0 | 10.9 | 11.0 | 1.25× |
| `bits/read_unaligned_16` | 14.5 | 3.7 | 3.7 | 4.9 | 4.3 | 3.7 | 3.7 | 4.0 | 4.0 | 3.62× |
| `bits/write_unaligned_1200` | 647 | 36.2 | 36.2 | 36.3 | 34.7 | 30.9 | 30.8 | 34.6 | 33.8 | 19.14× |
| `bits/read_unaligned_1200` | 637 | 22.1 | 21.8 | 22.9 | 21.9 | 16.4 | 16.1 | 17.7 | 19.2 | 33.19× |
| `bits/write_unaligned_16000` | 8,419 | 430 | 436 | 428 | 430 | 343 | 342 | 348 | 345 | 24.42× |
| `bits/read_unaligned_16000` | 8,304 | 298 | 299 | 300 | 298 | 222 | 217 | 225 | 214 | 38.86× |
| `fields/write_256_mixed_ints` | 1,786 | 1,537 | 1,340 | 1,345 | 933 | 943 | 938 | 428 | 420 | 4.25× |
| `fields/write_256_mixed_ints_slice` | 1,106 | 789 | 792 | 795 | 685 | 683 | 684 | 680 | 682 | 1.62× |
| `fields/read_256_mixed_ints` | 1,945 | 297 | 310 | 306 | 149 | 147 | 148 | 146 | 150 | 12.99× |

### x86_64 (Rosetta)

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 560 | 416 | 349 | 388 | 339 | 326 | 327 | 181 | 179 | 3.13× |
| `datagram/video_fragment/decode` | 548 | 138 | 139 | 158 | 152 | 150 | 151 | 151 | 152 | 3.61× |
| `datagram/transport_feedback_64/encode` | 5,756 | 4,346 | 3,449 | 3,890 | 3,281 | 3,153 | 3,215 | 1,898 | 1,901 | 3.03× |
| `datagram/transport_feedback_64/decode` | 7,250 | 2,136 | 1,964 | 2,354 | 1,654 | 1,666 | 1,662 | 1,653 | 1,625 | 4.46× |
| `datagram/padding_64/encode` | 239 | 159 | 137 | 150 | 135 | 131 | 134 | 101 | 102 | 2.36× |
| `datagram/padding_64/decode` | 231 | 59.9 | 61.6 | 71.4 | 70.5 | 66.2 | 68.6 | 66.4 | 66.4 | 3.48× |
| `datagram/padding_1200/encode` | 1,751 | 236 | 214 | 228 | 211 | 212 | 212 | 164 | 165 | 10.64× |
| `datagram/padding_1200/decode` | 1,792 | 143 | 143 | 152 | 150 | 154 | 153 | 136 | 136 | 13.15× |
| `datagram/padding_16000/encode` | 21,037 | 1,108 | 1,082 | 1,098 | 1,077 | 1,070 | 1,075 | 1,038 | 1,040 | 20.24× |
| `datagram/padding_16000/decode` | 21,504 | 1,006 | 1,006 | 1,018 | 1,015 | 1,015 | 1,017 | 1,002 | 1,003 | 21.45× |
| `strings/profile/encode` | 952 | 846 | 604 | 726 | 624 | 618 | 625 | 472 | 472 | 2.02× |
| `strings/profile/decode` | 877 | 187 | 188 | 203 | 190 | 185 | 186 | 198 | 201 | 4.36× |
| `bits/write_unaligned_16` | 31.6 | 25.3 | 20.9 | 23.2 | 21.2 | 22.4 | 21.4 | 22.4 | 22.4 | 1.41× |
| `bits/read_unaligned_16` | 37.1 | 7.2 | 7.0 | 8.6 | 7.6 | 6.8 | 7.1 | 8.3 | 8.4 | 4.42× |
| `bits/write_unaligned_1200` | 1,614 | 81.4 | 76.6 | 80.9 | 78.8 | 82.2 | 80.2 | 80.5 | 80.3 | 20.09× |
| `bits/read_unaligned_1200` | 1,609 | 61.5 | 61.1 | 61.9 | 61.7 | 62.5 | 62.5 | 62.4 | 62.4 | 25.79× |
| `bits/write_unaligned_16000` | 21,371 | 956 | 946 | 951 | 948 | 949 | 947 | 950 | 950 | 22.50× |
| `bits/read_unaligned_16000` | 21,203 | 818 | 815 | 819 | 819 | 819 | 819 | 820 | 821 | 25.84× |
| `fields/write_256_mixed_ints` | 3,659 | 3,061 | 2,120 | 2,503 | 1,578 | 1,581 | 1,583 | 1,215 | 1,212 | 3.02× |
| `fields/write_256_mixed_ints_slice` | 2,348 | 1,444 | 1,369 | 1,416 | 1,370 | 1,361 | 1,367 | 1,378 | 1,372 | 1.71× |
| `fields/read_256_mixed_ints` | 4,752 | 545 | 373 | 436 | 279 | 282 | 283 | 270 | 269 | 17.64× |

### wasm32

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 496 | 319 | 318 | 272 | 221 | 223 | 222 | 207 | 196 | 2.53× |
| `datagram/video_fragment/decode` | 355 | 251 | 252 | 266 | 174 | 173 | 173 | 173 | 173 | 2.05× |
| `datagram/transport_feedback_64/encode` | 4,958 | 3,269 | 2,933 | 2,451 | 1,931 | 1,944 | 1,952 | 2,025 | 1,958 | 2.53× |
| `datagram/transport_feedback_64/decode` | 4,494 | 2,777 | 2,952 | 2,851 | 1,891 | 1,892 | 1,903 | 1,905 | 1,876 | 2.40× |
| `datagram/padding_64/encode` | 191 | 134 | 124 | 114 | 102 | 100 | 101 | 105 | 101 | 1.89× |
| `datagram/padding_64/decode` | 157 | 90.4 | 89.6 | 99.4 | 74.9 | 74.6 | 74.6 | 74.0 | 75.3 | 2.09× |
| `datagram/padding_1200/encode` | 1,118 | 215 | 203 | 193 | 179 | 178 | 180 | 183 | 178 | 6.27× |
| `datagram/padding_1200/decode` | 1,080 | 166 | 166 | 176 | 151 | 150 | 151 | 150 | 151 | 7.16× |
| `datagram/padding_16000/encode` | 12,994 | 1,217 | 1,203 | 1,188 | 1,182 | 1,180 | 1,182 | 1,190 | 1,183 | 10.98× |
| `datagram/padding_16000/decode` | 12,932 | 1,174 | 1,172 | 1,185 | 1,168 | 1,171 | 1,160 | 1,165 | 1,160 | 11.15× |
| `strings/profile/encode` | 773 | 612 | 466 | 468 | 402 | 391 | 392 | 487 | 356 | 2.17× |
| `strings/profile/decode` | 336 | 336 | 335 | 350 | 230 | 231 | 230 | 230 | 230 | 1.46× |
| `bits/write_unaligned_16` | 23.6 | 18.2 | 17.1 | 17.0 | 17.5 | 17.4 | 17.3 | 19.1 | 16.1 | 1.47× |
| `bits/read_unaligned_16` | 21.9 | 5.5 | 5.5 | 5.4 | 6.8 | 6.8 | 6.8 | 8.5 | 8.6 | 2.55× |
| `bits/write_unaligned_1200` | 975 | 96.3 | 92.8 | 92.9 | 94.4 | 94.2 | 93.5 | 95.6 | 93.4 | 10.44× |
| `bits/read_unaligned_1200` | 969 | 74.6 | 74.8 | 80.6 | 76.2 | 77.1 | 76.0 | 77.0 | 77.7 | 12.47× |
| `bits/write_unaligned_16000` | 12,831 | 1,083 | 1,075 | 1,074 | 1,087 | 1,084 | 1,092 | 1,084 | 1,080 | 11.88× |
| `bits/read_unaligned_16000` | 12,721 | 949 | 949 | 943 | 964 | 953 | 949 | 968 | 967 | 13.15× |
| `fields/write_256_mixed_ints` | 4,332 | 2,371 | 2,201 | 2,226 | 1,809 | 1,836 | 1,816 | 1,243 | 993 | 4.36× |
| `fields/write_256_mixed_ints_slice` | 2,741 | 1,585 | 1,578 | 1,574 | 1,532 | 1,534 | 1,528 | 1,529 | 696 | 3.94× |
| `fields/read_256_mixed_ints` | 2,900 | 610 | 612 | 614 | 442 | 446 | 439 | 444 | 442 | 6.56× |

### wasm32 +simd128

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 480 | 346 | 347 | 287 | 219 | 220 | 220 | 207 | 193 | 2.48× |
| `datagram/video_fragment/decode` | 337 | 254 | 255 | 266 | 174 | 174 | 174 | 174 | 174 | 1.93× |
| `datagram/transport_feedback_64/encode` | 4,827 | 3,653 | 3,290 | 2,649 | 1,837 | 1,886 | 1,848 | 1,978 | 1,910 | 2.53× |
| `datagram/transport_feedback_64/decode` | 4,520 | 2,802 | 2,830 | 2,839 | 1,824 | 1,816 | 1,832 | 1,833 | 1,822 | 2.48× |
| `datagram/padding_64/encode` | 192 | 137 | 127 | 114 | 98.2 | 98.4 | 98.4 | 102 | 97.6 | 1.96× |
| `datagram/padding_64/decode` | 159 | 87.7 | 88.1 | 96.6 | 72.4 | 72.1 | 72.3 | 71.7 | 72.6 | 2.18× |
| `datagram/padding_1200/encode` | 1,117 | 185 | 169 | 157 | 140 | 137 | 136 | 140 | 135 | 8.29× |
| `datagram/padding_1200/decode` | 1,082 | 127 | 127 | 135 | 109 | 106 | 107 | 108 | 106 | 10.16× |
| `datagram/padding_16000/encode` | 12,963 | 704 | 693 | 684 | 670 | 623 | 621 | 626 | 620 | 20.92× |
| `datagram/padding_16000/decode` | 12,949 | 655 | 654 | 668 | 644 | 596 | 596 | 596 | 593 | 21.84× |
| `strings/profile/encode` | 770 | 627 | 488 | 484 | 390 | 407 | 385 | 493 | 352 | 2.18× |
| `strings/profile/decode` | 334 | 359 | 357 | 371 | 230 | 230 | 230 | 231 | 229 | 1.46× |
| `bits/write_unaligned_16` | 23.5 | 19.3 | 18.2 | 18.4 | 17.7 | 18.1 | 18.4 | 19.6 | 17.2 | 1.37× |
| `bits/read_unaligned_16` | 21.8 | 5.5 | 5.5 | 5.6 | 6.5 | 6.9 | 6.8 | 8.6 | 8.5 | 2.56× |
| `bits/write_unaligned_1200` | 973 | 57.9 | 55.1 | 55.3 | 54.4 | 51.1 | 51.6 | 53.4 | 51.8 | 18.79× |
| `bits/read_unaligned_1200` | 967 | 37.3 | 37.3 | 37.4 | 38.3 | 34.1 | 34.1 | 35.0 | 35.0 | 27.63× |
| `bits/write_unaligned_16000` | 12,814 | 585 | 585 | 586 | 586 | 539 | 537 | 537 | 536 | 23.92× |
| `bits/read_unaligned_16000` | 12,702 | 449 | 449 | 450 | 449 | 404 | 404 | 404 | 405 | 31.38× |
| `fields/write_256_mixed_ints` | 4,345 | 2,542 | 2,478 | 2,459 | 1,798 | 1,825 | 1,864 | 1,246 | 977 | 4.45× |
| `fields/write_256_mixed_ints_slice` | 2,733 | 1,746 | 1,741 | 1,743 | 1,531 | 1,534 | 1,520 | 1,514 | 692 | 3.95× |
| `fields/read_256_mixed_ints` | 2,927 | 615 | 616 | 616 | 436 | 435 | 430 | 431 | 437 | 6.70× |

### Change 9

Measured in separate runs with the parent commit as the baseline (column 8), see
[The series](#the-series).

| Benchmark | arm64 8 | arm64 9 | x86_64 8 | x86_64 9 | wasm32 8 | wasm32 9 | +simd128 8 | +simd128 9 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 91.9 | 94.8 | 178 | 175 | 195 | 196 | 193 | 194 |
| `datagram/video_fragment/decode` | 93.1 | 95.9 | 149 | 154 | 173 | 173 | 174 | 175 |
| `datagram/transport_feedback_64/encode` | 1,166 | 1,180 | 1,909 | 1,868 | 1,933 | 1,983 | 1,890 | 1,909 |
| `datagram/transport_feedback_64/decode` | 1,104 | 1,105 | 1,644 | 1,653 | 1,894 | 1,820 | 1,831 | 1,823 |
| `datagram/padding_64/encode` | 71.1 | 71.6 | 102 | 99.7 | 101 | 102 | 97.8 | 99.0 |
| `datagram/padding_64/decode` | 36.1 | 36.6 | 65.6 | 65.0 | 74.9 | 75.1 | 72.9 | 72.3 |
| `datagram/padding_1200/encode` | 92.2 | 92.6 | 172 | 168 | 179 | 179 | 135 | 137 |
| `datagram/padding_1200/decode` | 63.5 | 65.8 | 135 | 132 | 152 | 151 | 107 | 107 |
| `datagram/padding_16000/encode` | 420 | 414 | 1,037 | 1,034 | 1,196 | 1,191 | 626 | 632 |
| `datagram/padding_16000/decode` | 388 | 391 | 1,006 | 1,002 | 1,174 | 1,166 | 599 | 599 |
| `strings/profile/encode` | 228 | 234 | 491 | 482 | 352 | 354 | 357 | 354 |
| `strings/profile/decode` | 119 | 119 | 190 | 188 | 230 | 228 | 229 | 229 |
| `bits/write_unaligned_16` | 10.7 | 10.7 | 21.9 | 23.3 | 16.2 | 16.2 | 16.7 | 17.1 |
| `bits/read_unaligned_16` | 3.8 | 4.0 | 8.0 | 7.5 | 8.7 | 8.2 | 8.5 | 8.5 |
| `bits/write_unaligned_1200` | 34.5 | 33.7 | 79.5 | 80.2 | 93.6 | 93.5 | 52.2 | 52.2 |
| `bits/read_unaligned_1200` | 16.8 | 19.1 | 62.6 | 62.3 | 78.8 | 76.8 | 35.3 | 35.2 |
| `bits/write_unaligned_16000` | 351 | 352 | 951 | 954 | 1,096 | 1,080 | 540 | 541 |
| `bits/read_unaligned_16000` | 215 | 214 | 825 | 820 | 973 | 966 | 406 | 406 |
| `fields/write_256_mixed_ints` | 420 | 440 | 1,245 | 1,216 | 982 | 990 | 990 | 968 |
| `fields/write_256_mixed_ints_slice` | 664 | 657 | 1,365 | 1,352 | 698 | 698 | 695 | 702 |
| `fields/read_256_mixed_ints` | 141 | 141 | 270 | 269 | 442 | 447 | 435 | 435 |

### `opt-level = "s"`

The medians from change 4, the last one before the vector kernels, on.

#### arm64

| Benchmark | 4 | 5 | 7 | 8 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 200 | 198 | 95.3 | 95.6 |
| `datagram/video_fragment/decode` | 72.8 | 72.8 | 72.4 | 72.4 |
| `datagram/transport_feedback_64/encode` | 1,803 | 1,823 | 1,132 | 1,132 |
| `datagram/transport_feedback_64/decode` | 995 | 997 | 996 | 997 |
| `datagram/padding_64/encode` | 111 | 97.4 | 76.8 | 77.3 |
| `datagram/padding_64/decode` | 44.7 | 34.1 | 32.9 | 33.0 |
| `datagram/padding_1200/encode` | 385 | 123 | 99.0 | 99.1 |
| `datagram/padding_1200/decode` | 323 | 60.6 | 63.5 | 63.6 |
| `datagram/padding_16000/encode` | 3,817 | 445 | 414 | 417 |
| `datagram/padding_16000/decode` | 3,677 | 382 | 381 | 386 |
| `strings/profile/encode` | 443 | 418 | 257 | 259 |
| `strings/profile/decode` | 144 | 147 | 145 | 146 |
| `bits/write_unaligned_16` | 15.7 | 14.7 | 12.2 | 12.2 |
| `bits/read_unaligned_16` | 8.0 | 3.4 | 3.4 | 3.4 |
| `bits/write_unaligned_1200` | 289 | 38.2 | 35.3 | 35.4 |
| `bits/read_unaligned_1200` | 280 | 17.7 | 16.3 | 19.8 |
| `bits/write_unaligned_16000` | 3,844 | 355 | 351 | 352 |
| `bits/read_unaligned_16000` | 3,725 | 214 | 214 | 215 |
| `fields/write_256_mixed_ints` | 1,316 | 1,282 | 423 | 422 |
| `fields/write_256_mixed_ints_slice` | 674 | 682 | 672 | 674 |
| `fields/read_256_mixed_ints` | 150 | 151 | 151 | 151 |

#### x86_64 (Rosetta)

| Benchmark | 4 | 5 | 7 | 8 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 365 | 381 | 195 | 194 |
| `datagram/video_fragment/decode` | 137 | 121 | 121 | 123 |
| `datagram/transport_feedback_64/encode` | 3,326 | 3,569 | 1,814 | 1,809 |
| `datagram/transport_feedback_64/decode` | 1,620 | 1,644 | 1,530 | 1,528 |
| `datagram/padding_64/encode` | 181 | 171 | 128 | 129 |
| `datagram/padding_64/decode` | 88.8 | 67.0 | 63.2 | 63.1 |
| `datagram/padding_1200/encode` | 546 | 249 | 190 | 190 |
| `datagram/padding_1200/decode` | 453 | 146 | 135 | 135 |
| `datagram/padding_16000/encode` | 4,975 | 1,113 | 1,061 | 1,060 |
| `datagram/padding_16000/decode` | 4,980 | 1,013 | 1,001 | 1,000 |
| `strings/profile/encode` | 710 | 768 | 502 | 501 |
| `strings/profile/decode` | 249 | 232 | 226 | 226 |
| `bits/write_unaligned_16` | 32.5 | 30.7 | 26.4 | 26.4 |
| `bits/read_unaligned_16` | 13.4 | 6.6 | 7.2 | 7.2 |
| `bits/write_unaligned_1200` | 394 | 86.5 | 82.8 | 83.0 |
| `bits/read_unaligned_1200` | 363 | 61.8 | 62.3 | 62.3 |
| `bits/write_unaligned_16000` | 4,901 | 957 | 955 | 955 |
| `bits/read_unaligned_16000` | 4,669 | 817 | 818 | 819 |
| `fields/write_256_mixed_ints` | 2,217 | 2,462 | 1,230 | 1,231 |
| `fields/write_256_mixed_ints_slice` | 1,355 | 1,354 | 1,355 | 1,363 |
| `fields/read_256_mixed_ints` | 203 | 204 | 311 | 310 |

#### wasm32 +simd128

| Benchmark | 4 | 5 | 7 | 8 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 343 | 344 | 220 | 212 |
| `datagram/video_fragment/decode` | 137 | 137 | 137 | 136 |
| `datagram/transport_feedback_64/encode` | 3,181 | 3,176 | 1,932 | 1,894 |
| `datagram/transport_feedback_64/decode` | 1,730 | 1,724 | 1,726 | 1,729 |
| `datagram/padding_64/encode` | 180 | 160 | 130 | 127 |
| `datagram/padding_64/decode` | 101 | 74.6 | 74.8 | 75.0 |
| `datagram/padding_1200/encode` | 726 | 200 | 166 | 164 |
| `datagram/padding_1200/decode` | 641 | 110 | 111 | 110 |
| `datagram/padding_16000/encode` | 7,739 | 684 | 652 | 648 |
| `datagram/padding_16000/decode` | 7,653 | 599 | 600 | 600 |
| `strings/profile/encode` | 681 | 666 | 510 | 388 |
| `strings/profile/decode` | 282 | 265 | 264 | 263 |
| `bits/write_unaligned_16` | 28.9 | 29.1 | 23.6 | 22.4 |
| `bits/read_unaligned_16` | 13.7 | 9.4 | 9.7 | 9.7 |
| `bits/write_unaligned_1200` | 590 | 63.4 | 58.4 | 57.2 |
| `bits/read_unaligned_1200` | 569 | 35.7 | 36.0 | 36.4 |
| `bits/write_unaligned_16000` | 7,609 | 548 | 540 | 541 |
| `bits/read_unaligned_16000` | 7,474 | 404 | 404 | 406 |
| `fields/write_256_mixed_ints` | 2,592 | 2,596 | 1,226 | 1,057 |
| `fields/write_256_mixed_ints_slice` | 1,524 | 1,522 | 1,520 | 744 |
| `fields/read_256_mixed_ints` | 454 | 450 | 453 | 453 |
