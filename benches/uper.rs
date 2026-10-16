//! UPER encode/decode benchmarks.
//!
//! This uses a minimal timing loop instead of a benchmark framework so that the same binary also
//! runs on `wasm32-wasip1` (e.g. through `node`'s WASI support). Run with
//!
//! ```text
//! cargo bench --bench uper [-- <filter>]
//! ```

use asn1rs::prelude::*;
use asn1rs::protocol::per::unaligned::buffer::BitBuffer;
use asn1rs::protocol::per::unaligned::{BitRead, BitWrite};
use asn1rs::protocol::per::{PackedRead, PackedWrite};
use std::hint::black_box;
use std::time::{Duration, Instant};

// A trimmed-down datagram schema, shaped like the metadata headers that a real-time media
// transport puts in front of every packet: many constrained integers of odd bit widths, booleans,
// nested sequences, a CHOICE, SEQUENCE OF and OCTET STRING payloads at unaligned bit positions.
asn_to_rust!(
    r"BenchDatagram DEFINITIONS AUTOMATIC TAGS ::=
    BEGIN

    ParticipantId ::= INTEGER (0..65535)
    StreamId ::= INTEGER (0..255)
    TrackId ::= INTEGER (0..255)

    Timestamp ::= SEQUENCE {
        seconds INTEGER (0..9223372036854775807),
        nanos INTEGER (0..4294967295)
    }

    DatagramHeader ::= SEQUENCE {
        sequenceNumber INTEGER (0..65535),
        wrappingBytesCounter INTEGER (0..16777215),
        sendTimeOffset INTEGER (0..274877906943),
        sendRealtimeOffset INTEGER (-524288..524287),
        systemClockSpin INTEGER (0..7)
    }

    TimingInfo ::= SEQUENCE {
        mediaTimeOffset INTEGER (0..274877906943),
        realtimeOffset INTEGER (-524288..524287),
        collectTimeOffset INTEGER (0..65535),
        preprocessTimeOffset INTEGER (0..65535),
        encodeTimeOffset INTEGER (0..65535),
        sendTimeOffset INTEGER (0..65535),
        firstRelayReceiveTimeOffset INTEGER (-32768..32767),
        lastRelaySendTimeOffset INTEGER (-32768..32767)
    }

    VideoFragmentMetadata ::= SEQUENCE {
        spatialIndex INTEGER (0..2),
        temporalIndex INTEGER (0..2)
    }

    VideoFragmentDatagramMetadata ::= SEQUENCE {
        header DatagramHeader,
        participantId ParticipantId,
        streamId StreamId,
        trackId TrackId,
        keyFrame BOOLEAN,
        senderSequenceNumber INTEGER (0..65535),
        sendAttemptNumber INTEGER (0..15),
        timingInfo TimingInfo,
        totalDataSize INTEGER (0..268435455),
        fragmentSize INTEGER (1..1791),
        fragmentNumber INTEGER (0..262143),
        metadata VideoFragmentMetadata
    }

    PaddingDatagram ::= SEQUENCE {
        header DatagramHeader,
        payload OCTET STRING
    }

    TransportFeedbackPacket ::= SEQUENCE {
        sequenceNumber INTEGER (0..65535),
        receiveRealtimeTimestamp Timestamp
    }

    TransportFeedbackDatagram ::= SEQUENCE {
        header DatagramHeader,
        packets SEQUENCE OF TransportFeedbackPacket
    }

    ClientRelayDatagram ::= CHOICE {
        videoFragment VideoFragmentDatagramMetadata,
        padding PaddingDatagram,
        transportFeedback TransportFeedbackDatagram
    }

    -- The media data of a fragment, which runs from the octet boundary after the metadata to the
    -- end of the datagram
    VideoFragmentDatagram ::= SEQUENCE {
        metadata VideoFragmentDatagramMetadata,
        data [PER: TRAILING] OCTET STRING
    }

    ClientRelayDatagramWithData ::= CHOICE {
        videoFragment VideoFragmentDatagram,
        padding PaddingDatagram,
        transportFeedback TransportFeedbackDatagram
    }

    Profile ::= SEQUENCE {
        name UTF8String,
        initials UTF8String OPTIONAL,
        avatarUrl UTF8String OPTIONAL,
        callsign IA5String
    }

    END"
);

fn header() -> DatagramHeader {
    DatagramHeader {
        sequence_number: 51_234,
        wrapping_bytes_counter: 12_345_678,
        send_time_offset: 98_765_432_109,
        send_realtime_offset: -123_456,
        system_clock_spin: 5,
    }
}

fn video_fragment() -> ClientRelayDatagram {
    ClientRelayDatagram::VideoFragment(VideoFragmentDatagramMetadata {
        header: header(),
        participant_id: ParticipantId(4_242),
        stream_id: StreamId(1),
        track_id: TrackId(3),
        key_frame: true,
        sender_sequence_number: 40_000,
        send_attempt_number: 2,
        timing_info: TimingInfo {
            media_time_offset: 98_765_000_001,
            realtime_offset: 300_123,
            collect_time_offset: 333,
            preprocess_time_offset: 120,
            encode_time_offset: 87,
            send_time_offset: 15,
            first_relay_receive_time_offset: -1_234,
            last_relay_send_time_offset: 4_321,
        },
        total_data_size: 123_456,
        fragment_size: 1_150,
        fragment_number: 107,
        metadata: VideoFragmentMetadata {
            spatial_index: 1,
            temporal_index: 2,
        },
    })
}

/// The same as [`ClientRelayDatagramWithData`], but it holds the data in a [`bytes::Bytes`].
#[cfg(feature = "bytes")]
#[asn(choice)]
#[derive(Debug, PartialEq)]
pub enum SharedClientRelayDatagram {
    #[asn(complex(SharedVideoFragmentDatagram, tag(UNIVERSAL(16))))]
    VideoFragment(SharedVideoFragmentDatagram),
    #[asn(complex(PaddingDatagram, tag(UNIVERSAL(16))))]
    Padding(PaddingDatagram),
    #[asn(complex(TransportFeedbackDatagram, tag(UNIVERSAL(16))))]
    TransportFeedback(TransportFeedbackDatagram),
}

/// The same as [`VideoFragmentDatagram`], but it holds the data in a [`bytes::Bytes`].
#[cfg(feature = "bytes")]
#[asn(sequence)]
#[derive(Debug, PartialEq)]
pub struct SharedVideoFragmentDatagram {
    #[asn(complex(VideoFragmentDatagramMetadata, tag(UNIVERSAL(16))))]
    pub metadata: VideoFragmentDatagramMetadata,
    #[asn(octet_string(trailing))]
    pub data: bytes::Bytes,
}

fn padding(len: usize) -> ClientRelayDatagram {
    ClientRelayDatagram::Padding(PaddingDatagram {
        header: header(),
        payload: (0..len).map(|i| (i * 31 + 7) as u8).collect(),
    })
}

fn transport_feedback(packets: usize) -> ClientRelayDatagram {
    ClientRelayDatagram::TransportFeedback(TransportFeedbackDatagram {
        header: header(),
        packets: (0..packets as u64)
            .map(|i| TransportFeedbackPacket {
                sequence_number: (1_000 + i) as u16,
                receive_realtime_timestamp: Timestamp {
                    seconds: 1_760_000_000 + i,
                    nanos: (i as u32).wrapping_mul(7_919_993) % 1_000_000_000,
                },
            })
            .collect(),
    })
}

fn profile() -> Profile {
    Profile {
        name: String::from("Ada Lovelace (she/her) ✨"),
        initials: Some(String::from("AL")),
        avatar_url: Some(String::from("https://example.com/avatars/ada-lovelace.png")),
        callsign: String::from("ANALYTICAL-ENGINE-1843-NOTE-G-BERNOULLI-NUMBERS-0123456789"),
    }
}

fn encode<T: Writable>(value: &T) -> Vec<u8> {
    let mut writer = UperWriter::default();
    writer.write(value).unwrap();
    writer.into_bytes_vec()
}

fn decode<T: Readable>(bytes: &[u8]) -> T {
    let mut reader = UperReader::from(Bits::from(bytes));
    reader.read::<T>().unwrap()
}

struct Bench {
    filter: Vec<String>,
    target: Duration,
}

impl Bench {
    fn run<R>(&self, name: &str, bytes: Option<usize>, mut f: impl FnMut() -> R) {
        if !self.filter.is_empty() && !self.filter.iter().any(|f| name.contains(f.as_str())) {
            return;
        }

        // Calibrate the batch size so that one sample takes roughly `target / SAMPLES`.
        const SAMPLES: usize = 25;
        let per_sample = self.target / SAMPLES as u32;
        let mut iters = 1u64;
        loop {
            let start = Instant::now();
            for _ in 0..iters {
                black_box(f());
            }
            let elapsed = start.elapsed();
            if elapsed >= per_sample / 2 {
                let scale = per_sample.as_secs_f64() / elapsed.as_secs_f64();
                iters = ((iters as f64 * scale) as u64).max(1);
                break;
            }
            iters *= 2;
        }

        let mut samples = (0..SAMPLES)
            .map(|_| {
                let start = Instant::now();
                for _ in 0..iters {
                    black_box(f());
                }
                start.elapsed().as_secs_f64() * 1e9 / iters as f64
            })
            .collect::<Vec<_>>();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let median = samples[SAMPLES / 2];
        let min = samples[0];
        let throughput = bytes
            .map(|b| format!("{:>9.2} GiB/s", b as f64 / median / 1.073_741_824))
            .unwrap_or_default();
        println!("{name:<50} {median:>12.1} ns {min:>12.1} ns (min) {throughput}");
    }
}

fn main() {
    let filter = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect::<Vec<_>>();
    let target = std::env::var("BENCH_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(500));
    let bench = Bench { filter, target };

    println!("{:<50} {:>15} {:>21}", "benchmark", "median", "best sample");

    let video = video_fragment();
    let video_bytes = encode(&video);
    bench.run("datagram/video_fragment/encode", None, || {
        encode(black_box(&video))
    });
    bench.run("datagram/video_fragment/decode", None, || {
        decode::<ClientRelayDatagram>(black_box(&video_bytes))
    });

    // A video fragment with 1200 octets of media data, as a datagram carries it
    #[cfg(feature = "bytes")]
    {
        let ClientRelayDatagram::VideoFragment(metadata) = video_fragment() else {
            unreachable!()
        };
        let data = (0..1_200)
            .map(|i| (i * 31 + 7) as u8)
            .collect::<bytes::Bytes>();
        let len = data.len();

        // Encode the metadata and append the data, decode the metadata and take the data from
        // the octets after it, as without `[PER: TRAILING]`
        let append = ClientRelayDatagram::VideoFragment(metadata.clone());
        let append_encode = || {
            let mut bytes = encode(black_box(&append));
            bytes.reserve_exact(data.len());
            bytes.extend_from_slice(black_box(&data));
            bytes
        };
        let append_bytes = bytes::Bytes::from(append_encode());
        bench.run(
            "datagram/video_fragment_1200/append/encode",
            Some(len),
            append_encode,
        );
        bench.run(
            "datagram/video_fragment_1200/append/decode",
            Some(len),
            || {
                let bytes = black_box(&append_bytes).clone();
                let mut reader = UperReader::from(Bits::from(&bytes[..]));
                let metadata = reader.read::<ClientRelayDatagram>().unwrap();
                let data = bytes.slice(bytes.len() - reader.bits_remaining() / 8..);
                (metadata, data)
            },
        );

        let trailing_vec = ClientRelayDatagramWithData::VideoFragment(VideoFragmentDatagram {
            metadata: metadata.clone(),
            data: data.to_vec(),
        });
        let trailing_vec_encode = || {
            let mut writer = UperWriter::with_capacity(64 + len);
            writer.write(black_box(&trailing_vec)).unwrap();
            writer.into_bytes_vec()
        };
        assert_eq!(&append_bytes[..], &trailing_vec_encode()[..]);
        bench.run(
            "datagram/video_fragment_1200/trailing_vec/encode",
            Some(len),
            trailing_vec_encode,
        );
        bench.run(
            "datagram/video_fragment_1200/trailing_vec/decode",
            Some(len),
            || decode::<ClientRelayDatagramWithData>(black_box(&append_bytes)),
        );

        let trailing_bytes =
            SharedClientRelayDatagram::VideoFragment(SharedVideoFragmentDatagram {
                metadata,
                data: data.clone(),
            });
        let trailing_bytes_encode = || {
            let mut writer = UperWriter::with_capacity(64 + len);
            writer.write(black_box(&trailing_bytes)).unwrap();
            writer.into_bytes_vec()
        };
        assert_eq!(&append_bytes[..], &trailing_bytes_encode()[..]);
        assert_eq!(
            trailing_bytes,
            UperReader::from(append_bytes.clone())
                .read::<SharedClientRelayDatagram>()
                .unwrap()
        );
        bench.run(
            "datagram/video_fragment_1200/trailing_bytes/encode",
            Some(len),
            trailing_bytes_encode,
        );
        bench.run(
            "datagram/video_fragment_1200/trailing_bytes/decode",
            Some(len),
            || {
                UperReader::from(black_box(&append_bytes).clone())
                    .read::<SharedClientRelayDatagram>()
                    .unwrap()
            },
        );
    }

    let feedback = transport_feedback(64);
    let feedback_bytes = encode(&feedback);
    bench.run("datagram/transport_feedback_64/encode", None, || {
        encode(black_box(&feedback))
    });
    bench.run("datagram/transport_feedback_64/decode", None, || {
        decode::<ClientRelayDatagram>(black_box(&feedback_bytes))
    });

    for len in [64, 1_200, 16_000] {
        let value = padding(len);
        let bytes = encode(&value);
        bench.run(&format!("datagram/padding_{len}/encode"), Some(len), || {
            encode(black_box(&value))
        });
        bench.run(&format!("datagram/padding_{len}/decode"), Some(len), || {
            decode::<ClientRelayDatagram>(black_box(&bytes))
        });
    }

    let profile = profile();
    let profile_bytes = encode(&profile);
    bench.run("strings/profile/encode", None, || {
        encode(black_box(&profile))
    });
    bench.run("strings/profile/decode", None, || {
        decode::<Profile>(black_box(&profile_bytes))
    });

    // The raw bit copy kernels, independent of any ASN.1 structure.
    for len in [16, 1_200, 16_000] {
        let src = (0..len).map(|i| (i * 31 + 7) as u8).collect::<Vec<u8>>();
        let mut buffer = BitBuffer::with_capacity(len + 1);
        bench.run(&format!("bits/write_unaligned_{len}"), Some(len), || {
            buffer.clear();
            buffer.write_bits_with_len(&[0], 3).unwrap();
            buffer.write_bits(black_box(&src)).unwrap();
        });

        let mut packed = BitBuffer::with_capacity(len + 1);
        packed.write_bits_with_len(&[0], 3).unwrap();
        packed.write_bits(&src).unwrap();
        let mut dst = vec![0u8; len];
        bench.run(&format!("bits/read_unaligned_{len}"), Some(len), || {
            let mut bits = Bits::from(black_box(packed.content()));
            bits.read_bits_with_len(&mut [0], 3).unwrap();
            bits.read_bits(&mut dst).unwrap();
        });
    }

    // Individual small fields, which dominate the per-message cost.
    let mut buffer = BitBuffer::with_capacity(1024);
    bench.run("fields/write_256_mixed_ints", None, || {
        buffer.clear();
        for i in 0..32u64 {
            buffer
                .write_constrained_whole_number(0, 7, (i % 8) as i64)
                .unwrap();
            buffer
                .write_constrained_whole_number(0, 65_535, (i * 1_000) as i64)
                .unwrap();
            buffer
                .write_constrained_whole_number(-524_288, 524_287, i as i64 - 9)
                .unwrap();
            buffer
                .write_constrained_whole_number(0, 274_877_906_943, (i << 30) as i64)
                .unwrap();
            buffer
                .write_constrained_whole_number(0, i64::MAX, (i << 50) as i64)
                .unwrap();
            buffer
                .write_constrained_whole_number(0, 16_777_215, (i << 18) as i64)
                .unwrap();
            buffer.write_bit(i % 3 == 0).unwrap();
            buffer
                .write_unconstrained_whole_number(i as i64 * 100_003)
                .unwrap();
        }
    });
    // The same writes into a fixed slice, which excludes the cost of growing a `BitBuffer`.
    let mut slice = vec![0u8; 1024];
    bench.run("fields/write_256_mixed_ints_slice", None, || {
        let mut pos = 0usize;
        let mut w = (&mut slice[..], &mut pos);
        for i in 0..32u64 {
            w.write_constrained_whole_number(0, 7, (i % 8) as i64)
                .unwrap();
            w.write_constrained_whole_number(0, 65_535, (i * 1_000) as i64)
                .unwrap();
            w.write_constrained_whole_number(-524_288, 524_287, i as i64 - 9)
                .unwrap();
            w.write_constrained_whole_number(0, 274_877_906_943, (i << 30) as i64)
                .unwrap();
            w.write_constrained_whole_number(0, i64::MAX, (i << 50) as i64)
                .unwrap();
            w.write_constrained_whole_number(0, 16_777_215, (i << 18) as i64)
                .unwrap();
            w.write_bit(i % 3 == 0).unwrap();
            w.write_unconstrained_whole_number(i as i64 * 100_003)
                .unwrap();
        }
        pos
    });
    let ints = buffer.content().to_vec();
    bench.run("fields/read_256_mixed_ints", None, || {
        let mut bits = Bits::from(black_box(&ints[..]));
        let mut sum = 0i64;
        for _ in 0..32u64 {
            sum = sum.wrapping_add(bits.read_constrained_whole_number(0, 7).unwrap());
            sum = sum.wrapping_add(bits.read_constrained_whole_number(0, 65_535).unwrap());
            sum = sum.wrapping_add(
                bits.read_constrained_whole_number(-524_288, 524_287)
                    .unwrap(),
            );
            sum = sum.wrapping_add(
                bits.read_constrained_whole_number(0, 274_877_906_943)
                    .unwrap(),
            );
            sum = sum.wrapping_add(bits.read_constrained_whole_number(0, i64::MAX).unwrap());
            sum = sum.wrapping_add(bits.read_constrained_whole_number(0, 16_777_215).unwrap());
            sum = sum.wrapping_add(bits.read_bit().unwrap() as i64);
            sum = sum.wrapping_add(bits.read_unconstrained_whole_number().unwrap());
        }
        sum
    });
}
