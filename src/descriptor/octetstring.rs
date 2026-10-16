use crate::descriptor::{ReadableType, Reader, WritableType, Writer};
use asn1rs_model::asn::Tag;
use core::marker::PhantomData;

/// An OCTET STRING whose value an `Octets` holds, a `Vec<u8>` by default.
pub struct OctetString<
    Constraint: self::Constraint = NoConstraint,
    Octets: OctetStringType = Vec<u8>,
>(PhantomData<(Constraint, Octets)>);

pub trait Constraint: super::common::Constraint {
    const MIN: Option<u64> = None;
    const MAX: Option<u64> = None;
    const EXTENSIBLE: bool = false;
    /// Whether the PER encoding instruction `[PER: TRAILING]` applies. In UPER, the octets then
    /// start at the next octet boundary and extend to the end of the encoding, without a length
    /// determinant. See [`asn1rs_model::asn::EncodingInstruction::Trailing`].
    const TRAILING: bool = false;
}

#[derive(Default)]
pub struct NoConstraint;
impl super::common::Constraint for NoConstraint {
    const TAG: Tag = Tag::DEFAULT_OCTET_STRING;
}
impl Constraint for NoConstraint {}

/// A Rust type that can hold the value of an OCTET STRING.
pub trait OctetStringType: Sized {
    /// The octets of the value.
    fn as_octets(&self) -> &[u8];

    /// The value of the octets in `octets`, without copying them if possible.
    fn from_vec(octets: Vec<u8>) -> Self;

    /// The value of a copy of `octets`.
    #[inline]
    fn from_octets(octets: &[u8]) -> Self {
        Self::from_vec(octets.to_vec())
    }

    /// The value of the octets in `range` of `shared`.
    ///
    /// Types that can refer to the memory of `shared` instead of copying it override this.
    #[cfg(feature = "bytes")]
    #[inline]
    fn from_shared(shared: &bytes::Bytes, range: core::ops::Range<usize>) -> Self {
        Self::from_octets(&shared[range])
    }
}

impl OctetStringType for Vec<u8> {
    #[inline]
    fn as_octets(&self) -> &[u8] {
        self.as_slice()
    }

    #[inline]
    fn from_vec(octets: Vec<u8>) -> Self {
        octets
    }
}

#[cfg(feature = "bytes")]
impl OctetStringType for bytes::Bytes {
    #[inline]
    fn as_octets(&self) -> &[u8] {
        self.as_ref()
    }

    #[inline]
    fn from_vec(octets: Vec<u8>) -> Self {
        bytes::Bytes::from(octets)
    }

    #[inline]
    fn from_octets(octets: &[u8]) -> Self {
        bytes::Bytes::copy_from_slice(octets)
    }

    #[inline]
    fn from_shared(shared: &bytes::Bytes, range: core::ops::Range<usize>) -> Self {
        shared.slice(range)
    }
}

impl<Constraint: self::Constraint, Octets: OctetStringType> WritableType
    for OctetString<Constraint, Octets>
{
    type Type = Octets;

    #[inline]
    fn write_value<W: Writer>(writer: &mut W, value: &Self::Type) -> Result<(), W::Error> {
        writer.write_octet_string::<Constraint>(value.as_octets())
    }
}

impl<Constraint: self::Constraint, Octets: OctetStringType> ReadableType
    for OctetString<Constraint, Octets>
{
    type Type = Octets;

    #[inline]
    fn read_value<R: Reader>(reader: &mut R) -> Result<Self::Type, <R as Reader>::Error> {
        reader.read_octet_string_as::<Constraint, Octets>()
    }
}
