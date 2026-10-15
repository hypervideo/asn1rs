# UPER performance

This document records the optimization of the UPER encoder and decoder for x86_64, arm64 and
WebAssembly. Every change adds a section that backs it with measurements. Experiments that did
not pay off are recorded as well, so that they need not be repeated.

All numbers are medians from `benches/uper.rs`, in nanoseconds per operation. Lower is better.
Speedups are `before / after`.

## Summary

From the original code to change 5 (Shift unaligned bit copies with SSE2, NEON and simd128):

| Benchmark | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 → 169 ns (1.94×) | 560 → 326 ns (1.72×) | 496 → 223 ns (2.22×) | 480 → 220 ns (2.18×) |
| `datagram/video_fragment/decode` | 220 → 93.6 ns (2.35×) | 548 → 150 ns (3.66×) | 355 → 173 ns (2.05×) | 337 → 174 ns (1.94×) |
| `datagram/transport_feedback_64/encode` | 3,076 → 1,619 ns (1.90×) | 5,756 → 3,153 ns (1.83×) | 4,958 → 1,944 ns (2.55×) | 4,827 → 1,886 ns (2.56×) |
| `datagram/transport_feedback_64/decode` | 2,860 → 1,103 ns (2.59×) | 7,250 → 1,666 ns (4.35×) | 4,494 → 1,892 ns (2.37×) | 4,520 → 1,816 ns (2.49×) |
| `datagram/padding_1200/encode` | 758 → 104 ns (7.31×) | 1,751 → 212 ns (8.26×) | 1,118 → 178 ns (6.28×) | 1,117 → 137 ns (8.18×) |
| `datagram/padding_1200/decode` | 716 → 64.8 ns (11.06×) | 1,792 → 154 ns (11.60×) | 1,080 → 150 ns (7.19×) | 1,082 → 106 ns (10.16×) |
| `strings/profile/encode` | 788 → 318 ns (2.48×) | 952 → 618 ns (1.54×) | 773 → 391 ns (1.98×) | 770 → 407 ns (1.89×) |
| `strings/profile/decode` | 253 → 120 ns (2.11×) | 877 → 185 ns (4.73×) | 336 → 231 ns (1.46×) | 334 → 230 ns (1.45×) |
| `bits/write_unaligned_1200` | 647 → 30.9 ns (20.94×) | 1,614 → 82.2 ns (19.63×) | 975 → 94.2 ns (10.35×) | 973 → 51.1 ns (19.05×) |
| `bits/read_unaligned_1200` | 637 → 16.4 ns (38.85×) | 1,609 → 62.5 ns (25.74×) | 969 → 77.1 ns (12.57×) | 967 → 34.1 ns (28.36×) |
| `fields/write_256_mixed_ints` | 1,786 → 943 ns (1.89×) | 3,659 → 1,581 ns (2.31×) | 4,332 → 1,836 ns (2.36×) | 4,345 → 1,825 ns (2.38×) |
| `fields/read_256_mixed_ints` | 1,945 → 147 ns (13.27×) | 4,752 → 282 ns (16.88×) | 2,900 → 446 ns (6.50×) | 2,927 → 435 ns (6.72×) |

The changes so far:

| | Change |
|---|---|
| 1 | Copy bits word-at-a-time instead of bit-by-bit |
| 2 | Grow `BitBuffer` with `Vec::resize` |
| 3 | Keep the extension header out of the per-field scope code |
| 4 | Read and write integers as `u64` instead of byte arrays |
| 5 | Shift unaligned bit copies with SSE2, NEON and simd128 |

The encoded bytes do not change. After every change, a differential test encoded 4000 random
datagrams and 4000 random string sequences (about 11.7 MB of output) and compared the output byte
for byte with that of the original code.

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

## Appendix: all results

The medians of the series in ns, for every commit so far, and the speedup from the
original code to the last change.

### arm64

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | total |
|---|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 | 240 | 218 | 218 | 171 | 169 | 1.94× |
| `datagram/video_fragment/decode` | 220 | 90.5 | 91.2 | 95.1 | 94.2 | 93.6 | 2.35× |
| `datagram/transport_feedback_64/encode` | 3,076 | 2,309 | 2,069 | 2,046 | 1,621 | 1,619 | 1.90× |
| `datagram/transport_feedback_64/decode` | 2,860 | 1,300 | 1,298 | 1,264 | 1,109 | 1,103 | 2.59× |
| `datagram/padding_64/encode` | 134 | 99.4 | 97.4 | 99.4 | 85.7 | 82.9 | 1.62× |
| `datagram/padding_64/decode` | 97.3 | 36.5 | 36.4 | 39.0 | 38.5 | 37.1 | 2.62× |
| `datagram/padding_1200/encode` | 758 | 128 | 122 | 123 | 107 | 104 | 7.31× |
| `datagram/padding_1200/decode` | 716 | 69.7 | 69.4 | 71.5 | 70.7 | 64.8 | 11.06× |
| `datagram/padding_16000/encode` | 8,447 | 534 | 521 | 521 | 520 | 426 | 19.85× |
| `datagram/padding_16000/decode` | 8,415 | 475 | 478 | 473 | 482 | 384 | 21.92× |
| `strings/profile/encode` | 788 | 468 | 397 | 409 | 316 | 318 | 2.48× |
| `strings/profile/decode` | 253 | 119 | 119 | 130 | 123 | 120 | 2.11× |
| `bits/write_unaligned_16` | 13.7 | 12.7 | 11.9 | 13.3 | 11.1 | 11.3 | 1.21× |
| `bits/read_unaligned_16` | 14.5 | 3.7 | 3.7 | 4.9 | 4.3 | 3.7 | 3.92× |
| `bits/write_unaligned_1200` | 647 | 36.2 | 36.2 | 36.3 | 34.7 | 30.9 | 20.94× |
| `bits/read_unaligned_1200` | 637 | 22.1 | 21.8 | 22.9 | 21.9 | 16.4 | 38.85× |
| `bits/write_unaligned_16000` | 8,419 | 430 | 436 | 428 | 430 | 343 | 24.56× |
| `bits/read_unaligned_16000` | 8,304 | 298 | 299 | 300 | 298 | 222 | 37.42× |
| `fields/write_256_mixed_ints` | 1,786 | 1,537 | 1,340 | 1,345 | 933 | 943 | 1.89× |
| `fields/read_256_mixed_ints` | 1,945 | 297 | 310 | 306 | 149 | 147 | 13.27× |

### x86_64 (Rosetta)

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | total |
|---|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 560 | 416 | 349 | 388 | 339 | 326 | 1.72× |
| `datagram/video_fragment/decode` | 548 | 138 | 139 | 158 | 152 | 150 | 3.66× |
| `datagram/transport_feedback_64/encode` | 5,756 | 4,346 | 3,449 | 3,890 | 3,281 | 3,153 | 1.83× |
| `datagram/transport_feedback_64/decode` | 7,250 | 2,136 | 1,964 | 2,354 | 1,654 | 1,666 | 4.35× |
| `datagram/padding_64/encode` | 239 | 159 | 137 | 150 | 135 | 131 | 1.83× |
| `datagram/padding_64/decode` | 231 | 59.9 | 61.6 | 71.4 | 70.5 | 66.2 | 3.49× |
| `datagram/padding_1200/encode` | 1,751 | 236 | 214 | 228 | 211 | 212 | 8.26× |
| `datagram/padding_1200/decode` | 1,792 | 143 | 143 | 152 | 150 | 154 | 11.60× |
| `datagram/padding_16000/encode` | 21,037 | 1,108 | 1,082 | 1,098 | 1,077 | 1,070 | 19.66× |
| `datagram/padding_16000/decode` | 21,504 | 1,006 | 1,006 | 1,018 | 1,015 | 1,015 | 21.19× |
| `strings/profile/encode` | 952 | 846 | 604 | 726 | 624 | 618 | 1.54× |
| `strings/profile/decode` | 877 | 187 | 188 | 203 | 190 | 185 | 4.73× |
| `bits/write_unaligned_16` | 31.6 | 25.3 | 20.9 | 23.2 | 21.2 | 22.4 | 1.41× |
| `bits/read_unaligned_16` | 37.1 | 7.2 | 7.0 | 8.6 | 7.6 | 6.8 | 5.46× |
| `bits/write_unaligned_1200` | 1,614 | 81.4 | 76.6 | 80.9 | 78.8 | 82.2 | 19.63× |
| `bits/read_unaligned_1200` | 1,609 | 61.5 | 61.1 | 61.9 | 61.7 | 62.5 | 25.74× |
| `bits/write_unaligned_16000` | 21,371 | 956 | 946 | 951 | 948 | 949 | 22.51× |
| `bits/read_unaligned_16000` | 21,203 | 818 | 815 | 819 | 819 | 819 | 25.89× |
| `fields/write_256_mixed_ints` | 3,659 | 3,061 | 2,120 | 2,503 | 1,578 | 1,581 | 2.31× |
| `fields/read_256_mixed_ints` | 4,752 | 545 | 373 | 436 | 279 | 282 | 16.88× |

### wasm32

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | total |
|---|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 496 | 319 | 318 | 272 | 221 | 223 | 2.22× |
| `datagram/video_fragment/decode` | 355 | 251 | 252 | 266 | 174 | 173 | 2.05× |
| `datagram/transport_feedback_64/encode` | 4,958 | 3,269 | 2,933 | 2,451 | 1,931 | 1,944 | 2.55× |
| `datagram/transport_feedback_64/decode` | 4,494 | 2,777 | 2,952 | 2,851 | 1,891 | 1,892 | 2.37× |
| `datagram/padding_64/encode` | 191 | 134 | 124 | 114 | 102 | 100 | 1.90× |
| `datagram/padding_64/decode` | 157 | 90.4 | 89.6 | 99.4 | 74.9 | 74.6 | 2.11× |
| `datagram/padding_1200/encode` | 1,118 | 215 | 203 | 193 | 179 | 178 | 6.28× |
| `datagram/padding_1200/decode` | 1,080 | 166 | 166 | 176 | 151 | 150 | 7.19× |
| `datagram/padding_16000/encode` | 12,994 | 1,217 | 1,203 | 1,188 | 1,182 | 1,180 | 11.01× |
| `datagram/padding_16000/decode` | 12,932 | 1,174 | 1,172 | 1,185 | 1,168 | 1,171 | 11.04× |
| `strings/profile/encode` | 773 | 612 | 466 | 468 | 402 | 391 | 1.98× |
| `strings/profile/decode` | 336 | 336 | 335 | 350 | 230 | 231 | 1.46× |
| `bits/write_unaligned_16` | 23.6 | 18.2 | 17.1 | 17.0 | 17.5 | 17.4 | 1.36× |
| `bits/read_unaligned_16` | 21.9 | 5.5 | 5.5 | 5.4 | 6.8 | 6.8 | 3.22× |
| `bits/write_unaligned_1200` | 975 | 96.3 | 92.8 | 92.9 | 94.4 | 94.2 | 10.35× |
| `bits/read_unaligned_1200` | 969 | 74.6 | 74.8 | 80.6 | 76.2 | 77.1 | 12.57× |
| `bits/write_unaligned_16000` | 12,831 | 1,083 | 1,075 | 1,074 | 1,087 | 1,084 | 11.84× |
| `bits/read_unaligned_16000` | 12,721 | 949 | 949 | 943 | 964 | 953 | 13.35× |
| `fields/write_256_mixed_ints` | 4,332 | 2,371 | 2,201 | 2,226 | 1,809 | 1,836 | 2.36× |
| `fields/read_256_mixed_ints` | 2,900 | 610 | 612 | 614 | 442 | 446 | 6.50× |

### wasm32 +simd128

| Benchmark | original | 1 | 2 | 3 | 4 | 5 | total |
|---|---:|---:|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 480 | 346 | 347 | 287 | 219 | 220 | 2.18× |
| `datagram/video_fragment/decode` | 337 | 254 | 255 | 266 | 174 | 174 | 1.94× |
| `datagram/transport_feedback_64/encode` | 4,827 | 3,653 | 3,290 | 2,649 | 1,837 | 1,886 | 2.56× |
| `datagram/transport_feedback_64/decode` | 4,520 | 2,802 | 2,830 | 2,839 | 1,824 | 1,816 | 2.49× |
| `datagram/padding_64/encode` | 192 | 137 | 127 | 114 | 98.2 | 98.4 | 1.95× |
| `datagram/padding_64/decode` | 159 | 87.7 | 88.1 | 96.6 | 72.4 | 72.1 | 2.20× |
| `datagram/padding_1200/encode` | 1,117 | 185 | 169 | 157 | 140 | 137 | 8.18× |
| `datagram/padding_1200/decode` | 1,082 | 127 | 127 | 135 | 109 | 106 | 10.16× |
| `datagram/padding_16000/encode` | 12,963 | 704 | 693 | 684 | 670 | 623 | 20.82× |
| `datagram/padding_16000/decode` | 12,949 | 655 | 654 | 668 | 644 | 596 | 21.73× |
| `strings/profile/encode` | 770 | 627 | 488 | 484 | 390 | 407 | 1.89× |
| `strings/profile/decode` | 334 | 359 | 357 | 371 | 230 | 230 | 1.45× |
| `bits/write_unaligned_16` | 23.5 | 19.3 | 18.2 | 18.4 | 17.7 | 18.1 | 1.30× |
| `bits/read_unaligned_16` | 21.8 | 5.5 | 5.5 | 5.6 | 6.5 | 6.9 | 3.16× |
| `bits/write_unaligned_1200` | 973 | 57.9 | 55.1 | 55.3 | 54.4 | 51.1 | 19.05× |
| `bits/read_unaligned_1200` | 967 | 37.3 | 37.3 | 37.4 | 38.3 | 34.1 | 28.36× |
| `bits/write_unaligned_16000` | 12,814 | 585 | 585 | 586 | 586 | 539 | 23.77× |
| `bits/read_unaligned_16000` | 12,702 | 449 | 449 | 450 | 449 | 404 | 31.47× |
| `fields/write_256_mixed_ints` | 4,345 | 2,542 | 2,478 | 2,459 | 1,798 | 1,825 | 2.38× |
| `fields/read_256_mixed_ints` | 2,927 | 615 | 616 | 616 | 436 | 435 | 6.72× |

### `opt-level = "s"`

The medians from change 4, the last one before the vector kernels, on.

#### arm64

| Benchmark | 4 | 5 |
|---|---:|---:|
| `datagram/video_fragment/encode` | 200 | 198 |
| `datagram/video_fragment/decode` | 72.8 | 72.8 |
| `datagram/transport_feedback_64/encode` | 1,803 | 1,823 |
| `datagram/transport_feedback_64/decode` | 995 | 997 |
| `datagram/padding_64/encode` | 111 | 97.4 |
| `datagram/padding_64/decode` | 44.7 | 34.1 |
| `datagram/padding_1200/encode` | 385 | 123 |
| `datagram/padding_1200/decode` | 323 | 60.6 |
| `datagram/padding_16000/encode` | 3,817 | 445 |
| `datagram/padding_16000/decode` | 3,677 | 382 |
| `strings/profile/encode` | 443 | 418 |
| `strings/profile/decode` | 144 | 147 |
| `bits/write_unaligned_16` | 15.7 | 14.7 |
| `bits/read_unaligned_16` | 8.0 | 3.4 |
| `bits/write_unaligned_1200` | 289 | 38.2 |
| `bits/read_unaligned_1200` | 280 | 17.7 |
| `bits/write_unaligned_16000` | 3,844 | 355 |
| `bits/read_unaligned_16000` | 3,725 | 214 |
| `fields/write_256_mixed_ints` | 1,316 | 1,282 |
| `fields/read_256_mixed_ints` | 150 | 151 |

#### x86_64 (Rosetta)

| Benchmark | 4 | 5 |
|---|---:|---:|
| `datagram/video_fragment/encode` | 365 | 381 |
| `datagram/video_fragment/decode` | 137 | 121 |
| `datagram/transport_feedback_64/encode` | 3,326 | 3,569 |
| `datagram/transport_feedback_64/decode` | 1,620 | 1,644 |
| `datagram/padding_64/encode` | 181 | 171 |
| `datagram/padding_64/decode` | 88.8 | 67.0 |
| `datagram/padding_1200/encode` | 546 | 249 |
| `datagram/padding_1200/decode` | 453 | 146 |
| `datagram/padding_16000/encode` | 4,975 | 1,113 |
| `datagram/padding_16000/decode` | 4,980 | 1,013 |
| `strings/profile/encode` | 710 | 768 |
| `strings/profile/decode` | 249 | 232 |
| `bits/write_unaligned_16` | 32.5 | 30.7 |
| `bits/read_unaligned_16` | 13.4 | 6.6 |
| `bits/write_unaligned_1200` | 394 | 86.5 |
| `bits/read_unaligned_1200` | 363 | 61.8 |
| `bits/write_unaligned_16000` | 4,901 | 957 |
| `bits/read_unaligned_16000` | 4,669 | 817 |
| `fields/write_256_mixed_ints` | 2,217 | 2,462 |
| `fields/read_256_mixed_ints` | 203 | 204 |

#### wasm32 +simd128

| Benchmark | 4 | 5 |
|---|---:|---:|
| `datagram/video_fragment/encode` | 343 | 344 |
| `datagram/video_fragment/decode` | 137 | 137 |
| `datagram/transport_feedback_64/encode` | 3,181 | 3,176 |
| `datagram/transport_feedback_64/decode` | 1,730 | 1,724 |
| `datagram/padding_64/encode` | 180 | 160 |
| `datagram/padding_64/decode` | 101 | 74.6 |
| `datagram/padding_1200/encode` | 726 | 200 |
| `datagram/padding_1200/decode` | 641 | 110 |
| `datagram/padding_16000/encode` | 7,739 | 684 |
| `datagram/padding_16000/decode` | 7,653 | 599 |
| `strings/profile/encode` | 681 | 666 |
| `strings/profile/decode` | 282 | 265 |
| `bits/write_unaligned_16` | 28.9 | 29.1 |
| `bits/read_unaligned_16` | 13.7 | 9.4 |
| `bits/write_unaligned_1200` | 590 | 63.4 |
| `bits/read_unaligned_1200` | 569 | 35.7 |
| `bits/write_unaligned_16000` | 7,609 | 548 |
| `bits/read_unaligned_16000` | 7,474 | 404 |
| `fields/write_256_mixed_ints` | 2,592 | 2,596 |
| `fields/read_256_mixed_ints` | 454 | 450 |
