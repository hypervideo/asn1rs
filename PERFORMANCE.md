# UPER performance

This document records the optimization of the UPER encoder and decoder for x86_64, arm64 and
WebAssembly. Every change adds a section that backs it with measurements. Experiments that did
not pay off are recorded as well, so that they need not be repeated.

All numbers are medians from `benches/uper.rs`, in nanoseconds per operation. Lower is better.
Speedups are `before / after`.

## Summary

From the original code to change 3 (Keep the extension header out of the per-field scope code):

| Benchmark | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 → 218 ns (1.50×) | 560 → 388 ns (1.44×) | 496 → 272 ns (1.82×) | 480 → 287 ns (1.67×) |
| `datagram/video_fragment/decode` | 220 → 95.1 ns (2.32×) | 548 → 158 ns (3.46×) | 355 → 266 ns (1.33×) | 337 → 266 ns (1.27×) |
| `datagram/transport_feedback_64/encode` | 3,076 → 2,046 ns (1.50×) | 5,756 → 3,890 ns (1.48×) | 4,958 → 2,451 ns (2.02×) | 4,827 → 2,649 ns (1.82×) |
| `datagram/transport_feedback_64/decode` | 2,860 → 1,264 ns (2.26×) | 7,250 → 2,354 ns (3.08×) | 4,494 → 2,851 ns (1.58×) | 4,520 → 2,839 ns (1.59×) |
| `datagram/padding_1200/encode` | 758 → 123 ns (6.16×) | 1,751 → 228 ns (7.68×) | 1,118 → 193 ns (5.81×) | 1,117 → 157 ns (7.11×) |
| `datagram/padding_1200/decode` | 716 → 71.5 ns (10.02×) | 1,792 → 152 ns (11.75×) | 1,080 → 176 ns (6.13×) | 1,082 → 135 ns (8.00×) |
| `strings/profile/encode` | 788 → 409 ns (1.93×) | 952 → 726 ns (1.31×) | 773 → 468 ns (1.65×) | 770 → 484 ns (1.59×) |
| `strings/profile/decode` | 253 → 130 ns (1.95×) | 877 → 203 ns (4.31×) | 336 → 350 ns (0.96×) | 334 → 371 ns (0.90×) |
| `bits/write_unaligned_1200` | 647 → 36.3 ns (17.83×) | 1,614 → 80.9 ns (19.95×) | 975 → 92.9 ns (10.50×) | 973 → 55.3 ns (17.60×) |
| `bits/read_unaligned_1200` | 637 → 22.9 ns (27.83×) | 1,609 → 61.9 ns (25.99×) | 969 → 80.6 ns (12.02×) | 967 → 37.4 ns (25.86×) |
| `fields/write_256_mixed_ints` | 1,786 → 1,345 ns (1.33×) | 3,659 → 2,503 ns (1.46×) | 4,332 → 2,226 ns (1.95×) | 4,345 → 2,459 ns (1.77×) |
| `fields/read_256_mixed_ints` | 1,945 → 306 ns (6.36×) | 4,752 → 436 ns (10.91×) | 2,900 → 614 ns (4.72×) | 2,927 → 616 ns (4.75×) |

The changes so far:

| | Change |
|---|---|
| 1 | Copy bits word-at-a-time instead of bit-by-bit |
| 2 | Grow `BitBuffer` with `Vec::resize` |
| 3 | Keep the extension header out of the per-field scope code |

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

## Findings not addressed

- `UperReader::read_whole_sub_slice` limits the readable length through
  `core::mem::replace(&mut self.bits.len(), write_position)`, which replaces a temporary. The
  length is never limited.
- `Bits::read_bits*` check the position against the underlying slice, not against the length set
  through `ScopedBitRead::set_len`.
- `Error::insufficient_data_in_source_buffer` and `Error::insufficient_space_in_destination_buffer`
  capture a backtrace, which makes decoders that probe for the end of the input slow on that
  path.

## Appendix: all results

The medians of the series in ns, for every commit so far, and the speedup from the
original code to the last change.

### arm64

| Benchmark | original | 1 | 2 | 3 | total |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 | 240 | 218 | 218 | 1.50× |
| `datagram/video_fragment/decode` | 220 | 90.5 | 91.2 | 95.1 | 2.32× |
| `datagram/transport_feedback_64/encode` | 3,076 | 2,309 | 2,069 | 2,046 | 1.50× |
| `datagram/transport_feedback_64/decode` | 2,860 | 1,300 | 1,298 | 1,264 | 2.26× |
| `datagram/padding_64/encode` | 134 | 99.4 | 97.4 | 99.4 | 1.35× |
| `datagram/padding_64/decode` | 97.3 | 36.5 | 36.4 | 39.0 | 2.49× |
| `datagram/padding_1200/encode` | 758 | 128 | 122 | 123 | 6.16× |
| `datagram/padding_1200/decode` | 716 | 69.7 | 69.4 | 71.5 | 10.02× |
| `datagram/padding_16000/encode` | 8,447 | 534 | 521 | 521 | 16.20× |
| `datagram/padding_16000/decode` | 8,415 | 475 | 478 | 473 | 17.81× |
| `strings/profile/encode` | 788 | 468 | 397 | 409 | 1.93× |
| `strings/profile/decode` | 253 | 119 | 119 | 130 | 1.95× |
| `bits/write_unaligned_16` | 13.7 | 12.7 | 11.9 | 13.3 | 1.03× |
| `bits/read_unaligned_16` | 14.5 | 3.7 | 3.7 | 4.9 | 2.96× |
| `bits/write_unaligned_1200` | 647 | 36.2 | 36.2 | 36.3 | 17.83× |
| `bits/read_unaligned_1200` | 637 | 22.1 | 21.8 | 22.9 | 27.83× |
| `bits/write_unaligned_16000` | 8,419 | 430 | 436 | 428 | 19.69× |
| `bits/read_unaligned_16000` | 8,304 | 298 | 299 | 300 | 27.71× |
| `fields/write_256_mixed_ints` | 1,786 | 1,537 | 1,340 | 1,345 | 1.33× |
| `fields/read_256_mixed_ints` | 1,945 | 297 | 310 | 306 | 6.36× |

### x86_64 (Rosetta)

| Benchmark | original | 1 | 2 | 3 | total |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 560 | 416 | 349 | 388 | 1.44× |
| `datagram/video_fragment/decode` | 548 | 138 | 139 | 158 | 3.46× |
| `datagram/transport_feedback_64/encode` | 5,756 | 4,346 | 3,449 | 3,890 | 1.48× |
| `datagram/transport_feedback_64/decode` | 7,250 | 2,136 | 1,964 | 2,354 | 3.08× |
| `datagram/padding_64/encode` | 239 | 159 | 137 | 150 | 1.59× |
| `datagram/padding_64/decode` | 231 | 59.9 | 61.6 | 71.4 | 3.24× |
| `datagram/padding_1200/encode` | 1,751 | 236 | 214 | 228 | 7.68× |
| `datagram/padding_1200/decode` | 1,792 | 143 | 143 | 152 | 11.75× |
| `datagram/padding_16000/encode` | 21,037 | 1,108 | 1,082 | 1,098 | 19.16× |
| `datagram/padding_16000/decode` | 21,504 | 1,006 | 1,006 | 1,018 | 21.12× |
| `strings/profile/encode` | 952 | 846 | 604 | 726 | 1.31× |
| `strings/profile/decode` | 877 | 187 | 188 | 203 | 4.31× |
| `bits/write_unaligned_16` | 31.6 | 25.3 | 20.9 | 23.2 | 1.36× |
| `bits/read_unaligned_16` | 37.1 | 7.2 | 7.0 | 8.6 | 4.31× |
| `bits/write_unaligned_1200` | 1,614 | 81.4 | 76.6 | 80.9 | 19.95× |
| `bits/read_unaligned_1200` | 1,609 | 61.5 | 61.1 | 61.9 | 25.99× |
| `bits/write_unaligned_16000` | 21,371 | 956 | 946 | 951 | 22.47× |
| `bits/read_unaligned_16000` | 21,203 | 818 | 815 | 819 | 25.89× |
| `fields/write_256_mixed_ints` | 3,659 | 3,061 | 2,120 | 2,503 | 1.46× |
| `fields/read_256_mixed_ints` | 4,752 | 545 | 373 | 436 | 10.91× |

### wasm32

| Benchmark | original | 1 | 2 | 3 | total |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 496 | 319 | 318 | 272 | 1.82× |
| `datagram/video_fragment/decode` | 355 | 251 | 252 | 266 | 1.33× |
| `datagram/transport_feedback_64/encode` | 4,958 | 3,269 | 2,933 | 2,451 | 2.02× |
| `datagram/transport_feedback_64/decode` | 4,494 | 2,777 | 2,952 | 2,851 | 1.58× |
| `datagram/padding_64/encode` | 191 | 134 | 124 | 114 | 1.68× |
| `datagram/padding_64/decode` | 157 | 90.4 | 89.6 | 99.4 | 1.58× |
| `datagram/padding_1200/encode` | 1,118 | 215 | 203 | 193 | 5.81× |
| `datagram/padding_1200/decode` | 1,080 | 166 | 166 | 176 | 6.13× |
| `datagram/padding_16000/encode` | 12,994 | 1,217 | 1,203 | 1,188 | 10.94× |
| `datagram/padding_16000/decode` | 12,932 | 1,174 | 1,172 | 1,185 | 10.91× |
| `strings/profile/encode` | 773 | 612 | 466 | 468 | 1.65× |
| `strings/profile/decode` | 336 | 336 | 335 | 350 | 0.96× |
| `bits/write_unaligned_16` | 23.6 | 18.2 | 17.1 | 17.0 | 1.39× |
| `bits/read_unaligned_16` | 21.9 | 5.5 | 5.5 | 5.4 | 4.06× |
| `bits/write_unaligned_1200` | 975 | 96.3 | 92.8 | 92.9 | 10.50× |
| `bits/read_unaligned_1200` | 969 | 74.6 | 74.8 | 80.6 | 12.02× |
| `bits/write_unaligned_16000` | 12,831 | 1,083 | 1,075 | 1,074 | 11.95× |
| `bits/read_unaligned_16000` | 12,721 | 949 | 949 | 943 | 13.49× |
| `fields/write_256_mixed_ints` | 4,332 | 2,371 | 2,201 | 2,226 | 1.95× |
| `fields/read_256_mixed_ints` | 2,900 | 610 | 612 | 614 | 4.72× |

### wasm32 +simd128

| Benchmark | original | 1 | 2 | 3 | total |
|---|---:|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 480 | 346 | 347 | 287 | 1.67× |
| `datagram/video_fragment/decode` | 337 | 254 | 255 | 266 | 1.27× |
| `datagram/transport_feedback_64/encode` | 4,827 | 3,653 | 3,290 | 2,649 | 1.82× |
| `datagram/transport_feedback_64/decode` | 4,520 | 2,802 | 2,830 | 2,839 | 1.59× |
| `datagram/padding_64/encode` | 192 | 137 | 127 | 114 | 1.68× |
| `datagram/padding_64/decode` | 159 | 87.7 | 88.1 | 96.6 | 1.64× |
| `datagram/padding_1200/encode` | 1,117 | 185 | 169 | 157 | 7.11× |
| `datagram/padding_1200/decode` | 1,082 | 127 | 127 | 135 | 8.00× |
| `datagram/padding_16000/encode` | 12,963 | 704 | 693 | 684 | 18.96× |
| `datagram/padding_16000/decode` | 12,949 | 655 | 654 | 668 | 19.38× |
| `strings/profile/encode` | 770 | 627 | 488 | 484 | 1.59× |
| `strings/profile/decode` | 334 | 359 | 357 | 371 | 0.90× |
| `bits/write_unaligned_16` | 23.5 | 19.3 | 18.2 | 18.4 | 1.28× |
| `bits/read_unaligned_16` | 21.8 | 5.5 | 5.5 | 5.6 | 3.89× |
| `bits/write_unaligned_1200` | 973 | 57.9 | 55.1 | 55.3 | 17.60× |
| `bits/read_unaligned_1200` | 967 | 37.3 | 37.3 | 37.4 | 25.86× |
| `bits/write_unaligned_16000` | 12,814 | 585 | 585 | 586 | 21.87× |
| `bits/read_unaligned_16000` | 12,702 | 449 | 449 | 450 | 28.24× |
| `fields/write_256_mixed_ints` | 4,345 | 2,542 | 2,478 | 2,459 | 1.77× |
| `fields/read_256_mixed_ints` | 2,927 | 615 | 616 | 616 | 4.75× |
