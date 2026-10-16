mod test_utils;

use asn1rs::descriptor::{ReadableType, WritableType};
use asn1rs::macros::asn;
#[cfg(feature = "protobuf")]
use test_utils::{serialize_and_deserialize_protobuf, serialize_protobuf};
use test_utils::{serialize_and_deserialize_uper, serialize_uper};

#[asn(sequence)]
#[derive(Debug, Default, PartialEq)]
pub struct WithSkippedField {
    #[asn(integer(0..255))]
    pub first: u8,
    #[asn(skip(String))]
    pub skipped: String,
    #[asn(boolean)]
    pub last: bool,
}

#[test]
fn skipped_field_is_not_encoded_with_uper() {
    let value = WithSkippedField {
        first: 42,
        skipped: String::new(),
        last: true,
    };
    serialize_and_deserialize_uper(9, &[42, 0b1000_0000], &value);
    assert_eq!(
        serialize_uper(&value),
        serialize_uper(&WithSkippedField {
            skipped: "not encoded".to_string(),
            ..value
        })
    );
}

#[test]
#[cfg(feature = "protobuf")]
fn skipped_field_takes_no_protobuf_field_number() {
    let value = WithSkippedField {
        first: 42,
        skipped: String::new(),
        last: true,
    };
    // `last` is field 2, not 3
    serialize_and_deserialize_protobuf(&[8, 42, 16, 1], &value);
    assert_eq!(
        serialize_protobuf(&value),
        serialize_protobuf(&WithSkippedField {
            skipped: "not encoded".to_string(),
            ..value
        })
    );
}
