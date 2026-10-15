# UPER performance

This document records the optimization of the UPER encoder and decoder for x86_64, arm64 and
WebAssembly. Every change adds a section that backs it with measurements. Experiments that did
not pay off are recorded as well, so that they need not be repeated.

All numbers are medians from `benches/uper.rs`, in nanoseconds per operation. Lower is better.
Speedups are `before / after`.

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

The medians of the series in ns, for every commit so far.

### arm64

| Benchmark | original |
|---|---:|
| `datagram/video_fragment/encode` | 328 |
| `datagram/video_fragment/decode` | 220 |
| `datagram/transport_feedback_64/encode` | 3,076 |
| `datagram/transport_feedback_64/decode` | 2,860 |
| `datagram/padding_64/encode` | 134 |
| `datagram/padding_64/decode` | 97.3 |
| `datagram/padding_1200/encode` | 758 |
| `datagram/padding_1200/decode` | 716 |
| `datagram/padding_16000/encode` | 8,447 |
| `datagram/padding_16000/decode` | 8,415 |
| `strings/profile/encode` | 788 |
| `strings/profile/decode` | 253 |
| `bits/write_unaligned_16` | 13.7 |
| `bits/read_unaligned_16` | 14.5 |
| `bits/write_unaligned_1200` | 647 |
| `bits/read_unaligned_1200` | 637 |
| `bits/write_unaligned_16000` | 8,419 |
| `bits/read_unaligned_16000` | 8,304 |
| `fields/write_256_mixed_ints` | 1,786 |
| `fields/read_256_mixed_ints` | 1,945 |

### x86_64 (Rosetta)

| Benchmark | original |
|---|---:|
| `datagram/video_fragment/encode` | 560 |
| `datagram/video_fragment/decode` | 548 |
| `datagram/transport_feedback_64/encode` | 5,756 |
| `datagram/transport_feedback_64/decode` | 7,250 |
| `datagram/padding_64/encode` | 239 |
| `datagram/padding_64/decode` | 231 |
| `datagram/padding_1200/encode` | 1,751 |
| `datagram/padding_1200/decode` | 1,792 |
| `datagram/padding_16000/encode` | 21,037 |
| `datagram/padding_16000/decode` | 21,504 |
| `strings/profile/encode` | 952 |
| `strings/profile/decode` | 877 |
| `bits/write_unaligned_16` | 31.6 |
| `bits/read_unaligned_16` | 37.1 |
| `bits/write_unaligned_1200` | 1,614 |
| `bits/read_unaligned_1200` | 1,609 |
| `bits/write_unaligned_16000` | 21,371 |
| `bits/read_unaligned_16000` | 21,203 |
| `fields/write_256_mixed_ints` | 3,659 |
| `fields/read_256_mixed_ints` | 4,752 |

### wasm32

| Benchmark | original |
|---|---:|
| `datagram/video_fragment/encode` | 496 |
| `datagram/video_fragment/decode` | 355 |
| `datagram/transport_feedback_64/encode` | 4,958 |
| `datagram/transport_feedback_64/decode` | 4,494 |
| `datagram/padding_64/encode` | 191 |
| `datagram/padding_64/decode` | 157 |
| `datagram/padding_1200/encode` | 1,118 |
| `datagram/padding_1200/decode` | 1,080 |
| `datagram/padding_16000/encode` | 12,994 |
| `datagram/padding_16000/decode` | 12,932 |
| `strings/profile/encode` | 773 |
| `strings/profile/decode` | 336 |
| `bits/write_unaligned_16` | 23.6 |
| `bits/read_unaligned_16` | 21.9 |
| `bits/write_unaligned_1200` | 975 |
| `bits/read_unaligned_1200` | 969 |
| `bits/write_unaligned_16000` | 12,831 |
| `bits/read_unaligned_16000` | 12,721 |
| `fields/write_256_mixed_ints` | 4,332 |
| `fields/read_256_mixed_ints` | 2,900 |

### wasm32 +simd128

| Benchmark | original |
|---|---:|
| `datagram/video_fragment/encode` | 480 |
| `datagram/video_fragment/decode` | 337 |
| `datagram/transport_feedback_64/encode` | 4,827 |
| `datagram/transport_feedback_64/decode` | 4,520 |
| `datagram/padding_64/encode` | 192 |
| `datagram/padding_64/decode` | 159 |
| `datagram/padding_1200/encode` | 1,117 |
| `datagram/padding_1200/decode` | 1,082 |
| `datagram/padding_16000/encode` | 12,963 |
| `datagram/padding_16000/decode` | 12,949 |
| `strings/profile/encode` | 770 |
| `strings/profile/decode` | 334 |
| `bits/write_unaligned_16` | 23.5 |
| `bits/read_unaligned_16` | 21.8 |
| `bits/write_unaligned_1200` | 973 |
| `bits/read_unaligned_1200` | 967 |
| `bits/write_unaligned_16000` | 12,814 |
| `bits/read_unaligned_16000` | 12,702 |
| `fields/write_256_mixed_ints` | 4,345 |
| `fields/read_256_mixed_ints` | 2,927 |
