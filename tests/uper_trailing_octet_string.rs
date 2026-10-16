mod test_utils;

use asn1rs::descriptor::{Reader, Writer};
use asn1rs::macros::asn_to_rust;
use asn1rs::protocol::per::ErrorKind;
use asn1rs::rw::{Bits, UperReader, UperWriter};
use test_utils::{deserialize_uper, serialize_and_deserialize_uper, serialize_uper};

asn_to_rust!(
    r#"TrailingOctets DEFINITIONS AUTOMATIC TAGS ::=
    BEGIN

    Datagram ::= CHOICE {
        fragment Fragment,
        ping Ping
    }

    Fragment ::= SEQUENCE {
        number INTEGER (0..7),
        flag BOOLEAN,
        data [PER: TRAILING] OCTET STRING
    }

    Ping ::= SEQUENCE {
        payload OCTET STRING
    }

    -- What one would encode without the encoding instruction, before appending the octets
    DatagramWithoutData ::= CHOICE {
        fragment FragmentWithoutData,
        ping Ping
    }

    FragmentWithoutData ::= SEQUENCE {
        number INTEGER (0..7),
        flag BOOLEAN
    }

    Aligned ::= SEQUENCE {
        number INTEGER (0..255),
        data [PER: TRAILING] OCTET STRING
    }

    Bounded ::= SEQUENCE {
        flag BOOLEAN,
        data [PER: TRAILING] OCTET STRING (SIZE(1..4))
    }

    Payload ::= [PER: TRAILING] OCTET STRING

    NotLast ::= SEQUENCE {
        fragment Fragment,
        number INTEGER (0..7)
    }

    InExtension ::= SEQUENCE {
        number INTEGER (0..7),
        ...,
        data [PER: TRAILING] OCTET STRING
    }

    InChoiceExtension ::= CHOICE {
        number INTEGER (0..7),
        ...,
        payload Payload
    }

    END"#
);

#[test]
fn trailing_octets_start_at_the_next_octet_boundary() {
    // choice index 0, number 101, flag 1, three zero bits of padding
    serialize_and_deserialize_uper(
        4 * 8,
        &[0b0101_1000, 1, 2, 3],
        &Datagram::Fragment(Fragment {
            number: 5,
            flag: true,
            data: vec![1, 2, 3],
        }),
    );
}

#[test]
fn trailing_octets_follow_the_encoding_without_them() {
    let data = (0..=255).collect::<Vec<u8>>();
    let (_, mut without_data) =
        serialize_uper(&DatagramWithoutData::Fragment(FragmentWithoutData {
            number: 5,
            flag: true,
        }));
    without_data.extend_from_slice(&data);

    let (bits, with_data) = serialize_uper(&Datagram::Fragment(Fragment {
        number: 5,
        flag: true,
        data,
    }));
    assert_eq!(without_data, with_data);
    assert_eq!(with_data.len() * 8, bits);
}

#[test]
fn empty_trailing_octets_leave_only_the_padding() {
    serialize_and_deserialize_uper(
        8,
        &[0b0101_1000],
        &Datagram::Fragment(Fragment {
            number: 5,
            flag: true,
            data: Vec::new(),
        }),
    );
}

#[test]
fn aligned_trailing_octets_need_no_padding() {
    serialize_and_deserialize_uper(
        3 * 8,
        &[0xAB, 0xCD, 0xEF],
        &Aligned {
            number: 0xAB,
            data: vec![0xCD, 0xEF],
        },
    );
}

#[test]
fn trailing_octets_of_a_tuple_struct() {
    serialize_and_deserialize_uper(2 * 8, &[0xCA, 0xFE], &Payload(vec![0xCA, 0xFE]));
}

#[test]
fn trailing_octets_take_everything_after_the_boundary() {
    let mut reader = UperReader::from(Bits::from(&[0b0101_1111, 9, 8, 7, 6][..]));
    assert_eq!(
        Datagram::Fragment(Fragment {
            number: 5,
            flag: true,
            data: vec![9, 8, 7, 6],
        }),
        reader.read::<Datagram>().unwrap()
    );
    assert_eq!(0, reader.bits_remaining());
}

#[test]
fn other_alternatives_are_not_affected() {
    let ping = Datagram::Ping(Ping {
        payload: vec![1, 2, 3],
    });
    assert_eq!(
        serialize_uper(&DatagramWithoutData::Ping(Ping {
            payload: vec![1, 2, 3],
        })),
        serialize_uper(&ping)
    );
    let (bits, data) = serialize_uper(&ping);
    assert_eq!(ping, deserialize_uper::<Datagram>(&data, bits));
}

#[test]
fn trailing_octets_respect_the_size_constraint() {
    serialize_and_deserialize_uper(
        5 * 8,
        &[0x80, 1, 2, 3, 4],
        &Bounded {
            flag: true,
            data: vec![1, 2, 3, 4],
        },
    );

    let mut writer = UperWriter::default();
    let error = writer
        .write(&Bounded {
            flag: true,
            data: vec![1, 2, 3, 4, 5],
        })
        .unwrap_err();
    assert_eq!(&ErrorKind::SizeNotInRange(5, 1, 4), error.kind());

    let mut reader = UperReader::from(Bits::from(&[0x80][..]));
    let error = reader.read::<Bounded>().unwrap_err();
    assert_eq!(&ErrorKind::SizeNotInRange(0, 1, 4), error.kind());
}

#[test]
fn nothing_may_follow_trailing_octets() {
    let mut writer = UperWriter::default();
    let error = writer
        .write(&NotLast {
            fragment: Fragment {
                number: 1,
                flag: false,
                data: vec![1, 2],
            },
            number: 3,
        })
        .unwrap_err();
    assert_eq!(&ErrorKind::TrailingOctetStringNotLast, error.kind());
}

#[test]
fn trailing_octets_are_not_supported_in_extensions() {
    let mut writer = UperWriter::default();
    let error = writer
        .write(&InExtension {
            number: 1,
            data: Some(vec![1, 2]),
        })
        .unwrap_err();
    assert_eq!(&ErrorKind::TrailingOctetStringInOpenType, error.kind());

    let mut writer = UperWriter::default();
    let error = writer
        .write(&InChoiceExtension::Payload(Payload(vec![1, 2])))
        .unwrap_err();
    assert_eq!(&ErrorKind::TrailingOctetStringInOpenType, error.kind());
}

#[cfg(feature = "bytes")]
mod shared {
    use asn1rs::descriptor::{ReadableType, Reader, WritableType, Writer};
    use asn1rs::macros::asn;
    use asn1rs::rw::{Bits, UperReader, UperWriter};
    use bytes::Bytes;

    #[asn(sequence)]
    #[derive(Debug, Default, PartialEq)]
    pub struct SharedFragment {
        #[asn(integer(0..7))]
        number: u8,
        #[asn(boolean)]
        flag: bool,
        #[asn(octet_string(trailing))]
        data: Bytes,
    }

    #[asn(sequence)]
    #[derive(Debug, Default, PartialEq)]
    pub struct SharedPing {
        #[asn(integer(0..127))]
        number: u8,
        #[asn(octet_string)]
        payload: Bytes,
        #[asn(boolean)]
        flag: bool,
        #[asn(optional(octet_string))]
        extra: Option<Bytes>,
    }

    #[test]
    fn trailing_octets_are_shared_with_the_input() {
        let fragment = SharedFragment {
            number: 5,
            flag: true,
            data: Bytes::from_static(&[1, 2, 3]),
        };
        let mut writer = UperWriter::default();
        writer.write(&fragment).unwrap();
        let input = Bytes::from(writer.into_bytes_vec());
        assert_eq!(&[0b1011_0000, 1, 2, 3][..], &input[..]);

        let decoded = UperReader::from(input.clone())
            .read::<SharedFragment>()
            .unwrap();
        assert_eq!(fragment, decoded);
        assert!(input.as_ptr_range().contains(&decoded.data.as_ptr()));
    }

    #[test]
    fn aligned_octet_strings_are_shared_and_unaligned_ones_copied() {
        let ping = SharedPing {
            number: 7,
            payload: Bytes::from_static(&[4, 5, 6]),
            flag: true,
            extra: Some(Bytes::from_static(&[8, 9])),
        };
        let mut writer = UperWriter::default();
        writer.write(&ping).unwrap();
        let input = Bytes::from(writer.into_bytes_vec());

        // The presence bit of `extra` and `number` fill the first octet, so `payload` starts at
        // bit 16 after its length determinant. `flag` moves `extra` to bit 49.
        let decoded = UperReader::from(input.clone())
            .read::<SharedPing>()
            .unwrap();
        assert_eq!(ping, decoded);
        assert!(input.as_ptr_range().contains(&decoded.payload.as_ptr()));
        assert!(!input
            .as_ptr_range()
            .contains(&decoded.extra.as_ref().unwrap().as_ptr()));
    }

    #[test]
    fn bits_and_shared_bits_read_the_same() {
        let fragment = SharedFragment {
            number: 2,
            flag: false,
            data: Bytes::from_static(&[0xFF; 1200]),
        };
        let mut writer = UperWriter::default();
        writer.write(&fragment).unwrap();
        let input = writer.into_bytes_vec();

        let from_bits = UperReader::from(Bits::from(&input[..]))
            .read::<SharedFragment>()
            .unwrap();
        let from_shared = UperReader::from(Bytes::from(input))
            .read::<SharedFragment>()
            .unwrap();
        assert_eq!(fragment, from_bits);
        assert_eq!(fragment, from_shared);
    }
}
