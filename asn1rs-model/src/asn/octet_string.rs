use crate::asn::Size;
use crate::resolve::Error as ResolveError;
use crate::resolve::{LitOrRef, Resolver, TryResolve};
use std::fmt::{Debug, Display};

/// An OCTET STRING, ITU-T X.680 | ISO/IEC 8824-1, 23.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq)]
pub struct OctetString<SizeType: Display + Debug + Clone = usize> {
    /// The size constraint.
    pub size: Size<SizeType>,
    /// Whether the PER encoding instruction `[PER: TRAILING]` applies, see
    /// [`EncodingInstruction::Trailing`].
    pub trailing: bool,
}

impl<SizeType: Display + Debug + Clone> OctetString<SizeType> {
    /// An OCTET STRING with the size constraint `size` and without encoding instructions.
    pub const fn with_size(size: Size<SizeType>) -> Self {
        Self {
            size,
            trailing: false,
        }
    }
}

impl TryResolve<usize, OctetString<usize>> for OctetString<LitOrRef<usize>> {
    fn try_resolve(
        &self,
        resolver: &impl Resolver<usize>,
    ) -> Result<OctetString<usize>, ResolveError> {
        Ok(OctetString {
            size: self.size.try_resolve(resolver)?,
            trailing: self.trailing,
        })
    }
}

/// An encoding instruction in a type prefix, ITU-T X.680 | ISO/IEC 8824-1, 32.
///
/// ITU-T X.695 | ISO/IEC 8825-6 lets an ASN.1 specification apply PER encoding instructions so
/// that the UNALIGNED variant of PER reproduces the bits of an existing protocol. asn1rs
/// supports one such instruction, which is its own and is not registered under X.695.
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub enum EncodingInstruction {
    /// `[PER: TRAILING] OCTET STRING`
    ///
    /// In UPER, the OCTET STRING starts at the next octet boundary and extends to the end of the
    /// encoding. It has no length determinant, and the writer fills the bits up to the boundary
    /// with zeros. Nothing may be encoded after it, so it must be the last component of its
    /// SEQUENCE, and that SEQUENCE must in turn be the last thing in the encoding.
    ///
    /// Because the octets are aligned, they are copied without shifting, and a reader over shared
    /// memory can hand them out without copying.
    ///
    /// Other encoding rules encode the OCTET STRING as usual.
    Trailing,
}

impl EncodingInstruction {
    /// The name of [`EncodingInstruction::Trailing`] after the encoding reference `PER`.
    pub const PER_TRAILING: &'static str = "TRAILING";
}

#[cfg(test)]
mod tests {
    use super::OctetString;
    use crate::asn::{Asn, Size, Tag, Type};
    use crate::model::{Definition, Model};
    use crate::parse::{ErrorKind, Tokenizer};
    use crate::resolve::{LitOrRef, Unresolved};

    #[test]
    fn test_trailing_encoding_prefix() {
        let model = Model::<Asn<Unresolved>>::try_from(Tokenizer.parse(
            "Module DEFINITIONS AUTOMATIC TAGS ::= BEGIN
            Fragment ::= SEQUENCE {
                plain OCTET STRING,
                tagged [5] [PER: TRAILING] OCTET STRING (SIZE(0..9))
            }
            END",
        ))
        .unwrap();
        let Definition(_, asn) = &model.definitions[0];
        let Type::Sequence(sequence) = &asn.r#type else {
            panic!("Expected a SEQUENCE, got {:?}", asn.r#type);
        };
        assert_eq!(
            Type::OctetString(OctetString::with_size(Size::Any)),
            sequence.fields[0].role.r#type
        );
        assert_eq!(
            Type::OctetString(OctetString {
                size: Size::Range(LitOrRef::Lit(0), LitOrRef::Lit(9), false),
                trailing: true,
            }),
            sequence.fields[1].role.r#type
        );
        assert_eq!(Some(Tag::ContextSpecific(5)), sequence.fields[1].role.tag);
    }

    #[test]
    fn test_trailing_encoding_prefix_before_a_tag() {
        let model = Model::<Asn<Unresolved>>::try_from(Tokenizer.parse(
            "Module DEFINITIONS AUTOMATIC TAGS ::= BEGIN
            Fragment ::= SEQUENCE { data [PER:TRAILING] [APPLICATION 3] OCTET STRING }
            END",
        ))
        .unwrap();
        let Definition(_, asn) = &model.definitions[0];
        let Type::Sequence(sequence) = &asn.r#type else {
            panic!("Expected a SEQUENCE, got {:?}", asn.r#type);
        };
        assert_eq!(Some(Tag::Application(3)), sequence.fields[0].role.tag);
        assert_eq!(
            Type::OctetString(OctetString {
                size: Size::Any,
                trailing: true,
            }),
            sequence.fields[0].role.r#type
        );
    }

    #[test]
    fn test_trailing_encoding_prefix_of_a_definition_and_a_choice_alternative() {
        let model = Model::<Asn<Unresolved>>::try_from(Tokenizer.parse(
            "Module DEFINITIONS AUTOMATIC TAGS ::= BEGIN
            Payload ::= [PER: TRAILING] OCTET STRING
            Datagram ::= CHOICE { payload [PER: TRAILING] OCTET STRING }
            END",
        ))
        .unwrap();
        let trailing = Type::OctetString(OctetString {
            size: Size::Any,
            trailing: true,
        });
        assert_eq!(trailing, model.definitions[0].1.r#type);
        let Type::Choice(choice) = &model.definitions[1].1.r#type else {
            panic!("Expected a CHOICE, got {:?}", model.definitions[1].1.r#type);
        };
        assert_eq!(&trailing, choice.variants().next().unwrap().r#type());
    }

    #[test]
    fn test_unsupported_encoding_instructions() {
        for definition in [
            "A ::= SEQUENCE { data [PER: LEADING] OCTET STRING }",
            "A ::= SEQUENCE { data [XER: TRAILING] OCTET STRING }",
            "A ::= SEQUENCE { data [PER: TRAILING] [PER: TRAILING] OCTET STRING }",
        ] {
            let error = Model::<Asn<Unresolved>>::try_from(Tokenizer.parse(&format!(
                "Module DEFINITIONS AUTOMATIC TAGS ::= BEGIN {} END",
                definition
            )))
            .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::UnsupportedEncodingInstruction(_)),
                "{}: {}",
                definition,
                error
            );
        }
    }

    #[test]
    fn test_inapplicable_encoding_instructions() {
        for definition in [
            "A ::= SEQUENCE { data [PER: TRAILING] INTEGER }",
            "A ::= [PER: TRAILING] SEQUENCE { data OCTET STRING }",
            "A ::= CHOICE { data [PER: TRAILING] BOOLEAN }",
        ] {
            let error = Model::<Asn<Unresolved>>::try_from(Tokenizer.parse(&format!(
                "Module DEFINITIONS AUTOMATIC TAGS ::= BEGIN {} END",
                definition
            )))
            .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::InapplicableEncodingInstruction(_)),
                "{}: {}",
                definition,
                error
            );
        }
    }
}
