use super::bitcopy::{copy_bits, read_u64, write_u64};
use super::ensure_u64_bit_len;
use super::BitRead;
use crate::protocol::per::unaligned::BitWrite;
use crate::protocol::per::unaligned::BYTE_LEN;
use crate::protocol::per::{Error, ErrorKind};

impl BitRead for (&[u8], &mut usize) {
    #[inline]
    fn read_bit(&mut self) -> Result<bool, Error> {
        if *self.1 > self.0.len() * BYTE_LEN {
            return Err(ErrorKind::EndOfStream.into());
        }
        let bit = self.0[*self.1 / BYTE_LEN] & (0x80 >> (*self.1 % BYTE_LEN)) != 0;
        *self.1 += 1;
        Ok(bit)
    }

    #[inline]
    fn read_bits(&mut self, dst: &mut [u8]) -> Result<(), Error> {
        self.read_bits_with_offset_len(dst, 0, dst.len() * BYTE_LEN)
    }

    #[inline]
    fn read_bits_with_offset(
        &mut self,
        dst: &mut [u8],
        dst_bit_offset: usize,
    ) -> Result<(), Error> {
        self.read_bits_with_offset_len(dst, dst_bit_offset, dst.len() * BYTE_LEN - dst_bit_offset)
    }

    #[inline]
    fn read_bits_with_len(&mut self, dst: &mut [u8], dst_bit_len: usize) -> Result<(), Error> {
        self.read_bits_with_offset_len(dst, 0, dst_bit_len)
    }

    #[inline]
    fn read_bits_with_offset_len(
        &mut self,
        dst: &mut [u8],
        dst_bit_offset: usize,
        dst_bit_len: usize,
    ) -> Result<(), Error> {
        copy_bits(self.0, *self.1, dst, dst_bit_offset, dst_bit_len)?;
        *self.1 += dst_bit_len;
        Ok(())
    }

    #[inline]
    fn read_bits_u64(&mut self, bit_len: usize) -> Result<u64, Error> {
        ensure_u64_bit_len(bit_len)?;
        if self.0.len() * BYTE_LEN < *self.1 + bit_len {
            return Err(Error::insufficient_data_in_source_buffer());
        }
        let value = read_u64(self.0, *self.1, bit_len);
        *self.1 += bit_len;
        Ok(value)
    }
}

impl<'a> BitWrite for (&'a mut [u8], &mut usize) {
    #[inline]
    fn write_bit(&mut self, bit: bool) -> Result<(), Error> {
        if *self.1 + 1 > self.0.len() * BYTE_LEN {
            return Err(ErrorKind::EndOfStream.into());
        }
        if bit {
            self.0[*self.1 / BYTE_LEN] |= 0x80 >> (*self.1 % BYTE_LEN);
        } else {
            self.0[*self.1 / BYTE_LEN] &= !(0x80 >> (*self.1 % BYTE_LEN));
        }
        *self.1 += 1;
        Ok(())
    }

    #[inline]
    fn write_bits(&mut self, src: &[u8]) -> Result<(), Error> {
        self.write_bits_with_offset(src, 0)
    }

    #[inline]
    fn write_bits_with_offset(&mut self, src: &[u8], src_bit_offset: usize) -> Result<(), Error> {
        self.write_bits_with_offset_len(src, src_bit_offset, src.len() * BYTE_LEN - src_bit_offset)
    }

    #[inline]
    fn write_bits_with_len(&mut self, src: &[u8], bit_len: usize) -> Result<(), Error> {
        self.write_bits_with_offset_len(src, 0, bit_len)
    }

    #[inline]
    fn write_bits_with_offset_len(
        &mut self,
        src: &[u8],
        src_bit_offset: usize,
        src_bit_len: usize,
    ) -> Result<(), Error> {
        copy_bits(src, src_bit_offset, self.0, *self.1, src_bit_len)?;
        *self.1 += src_bit_len;
        Ok(())
    }

    #[inline]
    fn write_bits_u64(&mut self, value: u64, bit_len: usize) -> Result<(), Error> {
        ensure_u64_bit_len(bit_len)?;
        if self.0.len() * BYTE_LEN < *self.1 + bit_len {
            return Err(Error::insufficient_space_in_destination_buffer());
        }
        write_u64(self.0, *self.1, bit_len, value);
        *self.1 += bit_len;
        Ok(())
    }
}
