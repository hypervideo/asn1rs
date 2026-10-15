# UPER performance

This document records the optimization of the UPER encoder and decoder for x86_64, arm64 and
WebAssembly. Every change adds a section that backs it with measurements. Experiments that did
not pay off are recorded as well, so that they need not be repeated.

All numbers are medians from `benches/uper.rs`, in nanoseconds per operation. Lower is better.
Speedups are `before / after`.

## Summary

From the original code to change 1 (Copy bits word-at-a-time instead of bit-by-bit):

| Benchmark | arm64 | x86_64 (Rosetta) | wasm32 | wasm32 +simd128 |
|---|---:|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 → 240 ns (1.36×) | 560 → 416 ns (1.34×) | 496 → 319 ns (1.55×) | 480 → 346 ns (1.39×) |
| `datagram/video_fragment/decode` | 220 → 90.5 ns (2.43×) | 548 → 138 ns (3.96×) | 355 → 251 ns (1.42×) | 337 → 254 ns (1.32×) |
| `datagram/transport_feedback_64/encode` | 3,076 → 2,309 ns (1.33×) | 5,756 → 4,346 ns (1.32×) | 4,958 → 3,269 ns (1.52×) | 4,827 → 3,653 ns (1.32×) |
| `datagram/transport_feedback_64/decode` | 2,860 → 1,300 ns (2.20×) | 7,250 → 2,136 ns (3.39×) | 4,494 → 2,777 ns (1.62×) | 4,520 → 2,802 ns (1.61×) |
| `datagram/padding_1200/encode` | 758 → 128 ns (5.92×) | 1,751 → 236 ns (7.41×) | 1,118 → 215 ns (5.20×) | 1,117 → 185 ns (6.03×) |
| `datagram/padding_1200/decode` | 716 → 69.7 ns (10.28×) | 1,792 → 143 ns (12.56×) | 1,080 → 166 ns (6.52×) | 1,082 → 127 ns (8.55×) |
| `strings/profile/encode` | 788 → 468 ns (1.68×) | 952 → 846 ns (1.12×) | 773 → 612 ns (1.26×) | 770 → 627 ns (1.23×) |
| `strings/profile/decode` | 253 → 119 ns (2.13×) | 877 → 187 ns (4.69×) | 336 → 336 ns (1.00×) | 334 → 359 ns (0.93×) |
| `bits/write_unaligned_1200` | 647 → 36.2 ns (17.88×) | 1,614 → 81.4 ns (19.82×) | 975 → 96.3 ns (10.13×) | 973 → 57.9 ns (16.81×) |
| `bits/read_unaligned_1200` | 637 → 22.1 ns (28.83×) | 1,609 → 61.5 ns (26.16×) | 969 → 74.6 ns (12.99×) | 967 → 37.3 ns (25.92×) |
| `fields/write_256_mixed_ints` | 1,786 → 1,537 ns (1.16×) | 3,659 → 3,061 ns (1.20×) | 4,332 → 2,371 ns (1.83×) | 4,345 → 2,542 ns (1.71×) |
| `fields/read_256_mixed_ints` | 1,945 → 297 ns (6.55×) | 4,752 → 545 ns (8.72×) | 2,900 → 610 ns (4.75×) | 2,927 → 615 ns (4.76×) |

The changes so far:

| | Change |
|---|---|
| 1 | Copy bits word-at-a-time instead of bit-by-bit |

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

| Benchmark | original | 1 | total |
|---|---:|---:|---:|
| `datagram/video_fragment/encode` | 328 | 240 | 1.36× |
| `datagram/video_fragment/decode` | 220 | 90.5 | 2.43× |
| `datagram/transport_feedback_64/encode` | 3,076 | 2,309 | 1.33× |
| `datagram/transport_feedback_64/decode` | 2,860 | 1,300 | 2.20× |
| `datagram/padding_64/encode` | 134 | 99.4 | 1.35× |
| `datagram/padding_64/decode` | 97.3 | 36.5 | 2.67× |
| `datagram/padding_1200/encode` | 758 | 128 | 5.92× |
| `datagram/padding_1200/decode` | 716 | 69.7 | 10.28× |
| `datagram/padding_16000/encode` | 8,447 | 534 | 15.83× |
| `datagram/padding_16000/decode` | 8,415 | 475 | 17.72× |
| `strings/profile/encode` | 788 | 468 | 1.68× |
| `strings/profile/decode` | 253 | 119 | 2.13× |
| `bits/write_unaligned_16` | 13.7 | 12.7 | 1.08× |
| `bits/read_unaligned_16` | 14.5 | 3.7 | 3.92× |
| `bits/write_unaligned_1200` | 647 | 36.2 | 17.88× |
| `bits/read_unaligned_1200` | 637 | 22.1 | 28.83× |
| `bits/write_unaligned_16000` | 8,419 | 430 | 19.60× |
| `bits/read_unaligned_16000` | 8,304 | 298 | 27.83× |
| `fields/write_256_mixed_ints` | 1,786 | 1,537 | 1.16× |
| `fields/read_256_mixed_ints` | 1,945 | 297 | 6.55× |

### x86_64 (Rosetta)

| Benchmark | original | 1 | total |
|---|---:|---:|---:|
| `datagram/video_fragment/encode` | 560 | 416 | 1.34× |
| `datagram/video_fragment/decode` | 548 | 138 | 3.96× |
| `datagram/transport_feedback_64/encode` | 5,756 | 4,346 | 1.32× |
| `datagram/transport_feedback_64/decode` | 7,250 | 2,136 | 3.39× |
| `datagram/padding_64/encode` | 239 | 159 | 1.51× |
| `datagram/padding_64/decode` | 231 | 59.9 | 3.86× |
| `datagram/padding_1200/encode` | 1,751 | 236 | 7.41× |
| `datagram/padding_1200/decode` | 1,792 | 143 | 12.56× |
| `datagram/padding_16000/encode` | 21,037 | 1,108 | 18.99× |
| `datagram/padding_16000/decode` | 21,504 | 1,006 | 21.38× |
| `strings/profile/encode` | 952 | 846 | 1.12× |
| `strings/profile/decode` | 877 | 187 | 4.69× |
| `bits/write_unaligned_16` | 31.6 | 25.3 | 1.25× |
| `bits/read_unaligned_16` | 37.1 | 7.2 | 5.15× |
| `bits/write_unaligned_1200` | 1,614 | 81.4 | 19.82× |
| `bits/read_unaligned_1200` | 1,609 | 61.5 | 26.16× |
| `bits/write_unaligned_16000` | 21,371 | 956 | 22.35× |
| `bits/read_unaligned_16000` | 21,203 | 818 | 25.93× |
| `fields/write_256_mixed_ints` | 3,659 | 3,061 | 1.20× |
| `fields/read_256_mixed_ints` | 4,752 | 545 | 8.72× |

### wasm32

| Benchmark | original | 1 | total |
|---|---:|---:|---:|
| `datagram/video_fragment/encode` | 496 | 319 | 1.55× |
| `datagram/video_fragment/decode` | 355 | 251 | 1.42× |
| `datagram/transport_feedback_64/encode` | 4,958 | 3,269 | 1.52× |
| `datagram/transport_feedback_64/decode` | 4,494 | 2,777 | 1.62× |
| `datagram/padding_64/encode` | 191 | 134 | 1.42× |
| `datagram/padding_64/decode` | 157 | 90.4 | 1.74× |
| `datagram/padding_1200/encode` | 1,118 | 215 | 5.20× |
| `datagram/padding_1200/decode` | 1,080 | 166 | 6.52× |
| `datagram/padding_16000/encode` | 12,994 | 1,217 | 10.67× |
| `datagram/padding_16000/decode` | 12,932 | 1,174 | 11.02× |
| `strings/profile/encode` | 773 | 612 | 1.26× |
| `strings/profile/decode` | 336 | 336 | 1.00× |
| `bits/write_unaligned_16` | 23.6 | 18.2 | 1.30× |
| `bits/read_unaligned_16` | 21.9 | 5.5 | 3.98× |
| `bits/write_unaligned_1200` | 975 | 96.3 | 10.13× |
| `bits/read_unaligned_1200` | 969 | 74.6 | 12.99× |
| `bits/write_unaligned_16000` | 12,831 | 1,083 | 11.85× |
| `bits/read_unaligned_16000` | 12,721 | 949 | 13.41× |
| `fields/write_256_mixed_ints` | 4,332 | 2,371 | 1.83× |
| `fields/read_256_mixed_ints` | 2,900 | 610 | 4.75× |

### wasm32 +simd128

| Benchmark | original | 1 | total |
|---|---:|---:|---:|
| `datagram/video_fragment/encode` | 480 | 346 | 1.39× |
| `datagram/video_fragment/decode` | 337 | 254 | 1.32× |
| `datagram/transport_feedback_64/encode` | 4,827 | 3,653 | 1.32× |
| `datagram/transport_feedback_64/decode` | 4,520 | 2,802 | 1.61× |
| `datagram/padding_64/encode` | 192 | 137 | 1.40× |
| `datagram/padding_64/decode` | 159 | 87.7 | 1.81× |
| `datagram/padding_1200/encode` | 1,117 | 185 | 6.03× |
| `datagram/padding_1200/decode` | 1,082 | 127 | 8.55× |
| `datagram/padding_16000/encode` | 12,963 | 704 | 18.42× |
| `datagram/padding_16000/decode` | 12,949 | 655 | 19.77× |
| `strings/profile/encode` | 770 | 627 | 1.23× |
| `strings/profile/decode` | 334 | 359 | 0.93× |
| `bits/write_unaligned_16` | 23.5 | 19.3 | 1.22× |
| `bits/read_unaligned_16` | 21.8 | 5.5 | 3.96× |
| `bits/write_unaligned_1200` | 973 | 57.9 | 16.81× |
| `bits/read_unaligned_1200` | 967 | 37.3 | 25.92× |
| `bits/write_unaligned_16000` | 12,814 | 585 | 21.91× |
| `bits/read_unaligned_16000` | 12,702 | 449 | 28.30× |
| `fields/write_256_mixed_ints` | 4,345 | 2,542 | 1.71× |
| `fields/read_256_mixed_ints` | 2,927 | 615 | 4.76× |
