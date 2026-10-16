use super::{BitRead, ScopedBitRead, BYTE_LEN};
use crate::descriptor::octetstring::OctetStringType;
use crate::protocol::per::{Error, ErrorKind};
use bytes::Bytes;

/// Bits to read from a [`Bytes`], like [`super::buffer::Bits`] reads from a slice.
///
/// An OCTET STRING that starts at an octet boundary is read without copying when its Rust type
/// can share the memory of the [`Bytes`], as [`Bytes`] itself can.
#[derive(Debug, Clone)]
pub struct SharedBits {
    bytes: Bytes,
    pos: usize,
    len: usize,
}

impl SharedBits {
    /// The bytes that `self` reads from.
    pub fn into_bytes(self) -> Bytes {
        self.bytes
    }
}

impl From<Bytes> for SharedBits {
    fn from(bytes: Bytes) -> Self {
        let len = bytes.len() * BYTE_LEN;
        Self { bytes, pos: 0, len }
    }
}

impl From<(Bytes, usize)> for SharedBits {
    fn from((bytes, len): (Bytes, usize)) -> Self {
        debug_assert!(len <= bytes.len() * BYTE_LEN);
        Self { bytes, pos: 0, len }
    }
}

impl BitRead for SharedBits {
    #[inline]
    fn read_bit(&mut self) -> Result<bool, Error> {
        if self.pos < self.len {
            BitRead::read_bit(&mut (&self.bytes[..], &mut self.pos))
        } else {
            Err(ErrorKind::EndOfStream.into())
        }
    }

    #[inline]
    fn read_bits(&mut self, dst: &mut [u8]) -> Result<(), Error> {
        BitRead::read_bits(&mut (&self.bytes[..], &mut self.pos), dst)
    }

    #[inline]
    fn read_bits_with_offset(
        &mut self,
        dst: &mut [u8],
        dst_bit_offset: usize,
    ) -> Result<(), Error> {
        BitRead::read_bits_with_offset(&mut (&self.bytes[..], &mut self.pos), dst, dst_bit_offset)
    }

    #[inline]
    fn read_bits_with_len(&mut self, dst: &mut [u8], dst_bit_len: usize) -> Result<(), Error> {
        BitRead::read_bits_with_len(&mut (&self.bytes[..], &mut self.pos), dst, dst_bit_len)
    }

    #[inline]
    fn read_bits_with_offset_len(
        &mut self,
        dst: &mut [u8],
        dst_bit_offset: usize,
        dst_bit_len: usize,
    ) -> Result<(), Error> {
        BitRead::read_bits_with_offset_len(
            &mut (&self.bytes[..], &mut self.pos),
            dst,
            dst_bit_offset,
            dst_bit_len,
        )
    }

    #[inline]
    fn read_bits_u64(&mut self, bit_len: usize) -> Result<u64, Error> {
        BitRead::read_bits_u64(&mut (&self.bytes[..], &mut self.pos), bit_len)
    }

    #[inline]
    fn read_octets<Octets: OctetStringType>(&mut self, byte_len: usize) -> Result<Octets, Error> {
        if !self.pos.is_multiple_of(BYTE_LEN) {
            let mut octets = vec![0u8; byte_len];
            self.read_bits(&mut octets)?;
            return Ok(Octets::from_vec(octets));
        }
        let start = self.pos / BYTE_LEN;
        let end = start + byte_len;
        if end * BYTE_LEN > self.len {
            return Err(Error::insufficient_data_in_source_buffer());
        }
        self.pos = end * BYTE_LEN;
        Ok(Octets::from_shared(&self.bytes, start..end))
    }
}

impl ScopedBitRead for SharedBits {
    #[inline]
    fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    fn set_pos(&mut self, position: usize) -> usize {
        let pos = position.min(self.len);
        self.pos = pos;
        pos
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    #[inline]
    fn set_len(&mut self, len: usize) -> usize {
        let len = len.min(self.bytes.len() * BYTE_LEN);
        self.len = len;
        len
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.len - self.pos
    }
}
