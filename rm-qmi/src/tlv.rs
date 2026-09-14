use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

/// Byte order of a multi-byte field on the wire.
///
/// QMI fields are little-endian unless the database marks them as
/// `network`/`big`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

/// A borrowed TLV: a type byte and its raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub id: u8,
    pub value: &'a [u8],
}

impl<'a> Tlv<'a> {
    pub fn reader(self) -> TlvReader<'a> {
        TlvReader::new(self.value)
    }
}

/// An owned TLV, used when building messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlvBuf {
    pub id: u8,
    pub value: Vec<u8>,
}

/// Iterator over the TLVs of a message body.
pub struct TlvIter<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for TlvIter<'a> {
    type Item = Tlv<'a>;

    fn next(&mut self) -> Option<Tlv<'a>> {
        if self.rest.len() < 3 {
            self.rest = &[];
            return None;
        }

        let id = self.rest[0];
        let length = usize::from(u16::from_le_bytes([self.rest[1], self.rest[2]]));
        let body = &self.rest[3..];

        if body.len() < length {
            self.rest = &[];
            return None;
        }

        let (value, rest) = body.split_at(length);
        self.rest = rest;

        Some(Tlv { id, value })
    }
}

/// Iterate over the TLVs in a message body.
pub fn tlvs(body: &[u8]) -> TlvIter<'_> {
    TlvIter { rest: body }
}

/// Cursor over the value of a single TLV.
#[derive(Debug, Clone)]
pub struct TlvReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> TlvReader<'a> {
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub const fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Assert that the whole TLV value was consumed.
    pub fn finish(self, id: u8) -> Result<(), Error> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(Error::TrailingBytes { id })
        }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        if self.remaining() < length {
            return Err(Error::Truncated);
        }

        let out = &self.buf[self.pos..self.pos + length];
        self.pos += length;

        Ok(out)
    }

    pub fn read_u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8, Error> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_u16(&mut self, endian: Endian) -> Result<u16, Error> {
        let bytes: [u8; 2] = self.take(2)?.try_into().unwrap();
        Ok(match endian {
            Endian::Little => u16::from_le_bytes(bytes),
            Endian::Big => u16::from_be_bytes(bytes),
        })
    }

    pub fn read_u32(&mut self, endian: Endian) -> Result<u32, Error> {
        let bytes: [u8; 4] = self.take(4)?.try_into().unwrap();
        Ok(match endian {
            Endian::Little => u32::from_le_bytes(bytes),
            Endian::Big => u32::from_be_bytes(bytes),
        })
    }

    pub fn read_u64(&mut self, endian: Endian) -> Result<u64, Error> {
        let bytes: [u8; 8] = self.take(8)?.try_into().unwrap();
        Ok(match endian {
            Endian::Little => u64::from_le_bytes(bytes),
            Endian::Big => u64::from_be_bytes(bytes),
        })
    }

    pub fn read_i16(&mut self, endian: Endian) -> Result<i16, Error> {
        Ok(self.read_u16(endian)? as i16)
    }

    pub fn read_i32(&mut self, endian: Endian) -> Result<i32, Error> {
        Ok(self.read_u32(endian)? as i32)
    }

    pub fn read_i64(&mut self, endian: Endian) -> Result<i64, Error> {
        Ok(self.read_u64(endian)? as i64)
    }

    pub fn read_f32(&mut self, endian: Endian) -> Result<f32, Error> {
        Ok(f32::from_bits(self.read_u32(endian)?))
    }

    pub fn read_f64(&mut self, endian: Endian) -> Result<f64, Error> {
        Ok(f64::from_bits(self.read_u64(endian)?))
    }

    /// Read an integer of `bytes` length, as used by `guint-sized` fields.
    ///
    /// Mirrors `qmi_message_tlv_read_sized_guint()`: little-endian values are
    /// zero-extended from the low bytes, big-endian values from the high bytes.
    pub fn read_uint(&mut self, bytes: usize, endian: Endian) -> Result<u64, Error> {
        if bytes > 8 {
            return Err(Error::InvalidLength);
        }

        let raw = self.take(bytes)?;
        let mut tmp = [0u8; 8];

        match endian {
            Endian::Little => tmp[..bytes].copy_from_slice(raw),
            Endian::Big => tmp[8 - bytes..].copy_from_slice(raw),
        }

        Ok(match endian {
            Endian::Little => u64::from_le_bytes(tmp),
            Endian::Big => u64::from_be_bytes(tmp),
        })
    }

    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        self.take(length)
    }

    /// Read a variable-length string.
    ///
    /// `prefix_bytes` is the width of the length prefix (0, 1 or 2). When
    /// `max_size` is non-zero the value is truncated to it, matching
    /// `qmi_message_tlv_read_string()`.
    pub fn read_string(&mut self, prefix_bytes: u8, max_size: usize) -> Result<String, Error> {
        let length = match prefix_bytes {
            0 => self.remaining(),
            1 => usize::from(self.read_u8()?),
            2 => usize::from(self.read_u16(Endian::Little)?),
            _ => return Err(Error::InvalidLength),
        };

        let bytes = self.take(length)?;
        let valid = if max_size > 0 && length > max_size {
            &bytes[..max_size]
        } else {
            bytes
        };

        let end = valid
            .iter()
            .rposition(|&byte| byte != 0)
            .map_or(0, |i| i + 1);

        core::str::from_utf8(&valid[..end])
            .map(String::from)
            .map_err(|_| Error::InvalidString)
    }

    /// Read a fixed-size, NUL-padded string.
    pub fn read_fixed_string(&mut self, length: usize) -> Result<String, Error> {
        let bytes = self.take(length)?;
        let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(length);

        core::str::from_utf8(&bytes[..end])
            .map(String::from)
            .map_err(|_| Error::InvalidString)
    }
}

/// Builder for the value of a single TLV.
#[derive(Debug, Default, Clone)]
pub struct TlvWriter {
    buf: Vec<u8>,
}

impl TlvWriter {
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// Finish the TLV by attaching its type byte.
    pub fn into_tlv(self, id: u8) -> TlvBuf {
        TlvBuf {
            id,
            value: self.buf,
        }
    }

    pub fn write_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn write_i8(&mut self, value: i8) {
        self.buf.push(value as u8);
    }

    pub fn write_u16(&mut self, value: u16, endian: Endian) {
        self.buf.extend_from_slice(&match endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        });
    }

    pub fn write_u32(&mut self, value: u32, endian: Endian) {
        self.buf.extend_from_slice(&match endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        });
    }

    pub fn write_u64(&mut self, value: u64, endian: Endian) {
        self.buf.extend_from_slice(&match endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        });
    }

    pub fn write_i16(&mut self, value: i16, endian: Endian) {
        self.write_u16(value as u16, endian);
    }

    pub fn write_i32(&mut self, value: i32, endian: Endian) {
        self.write_u32(value as u32, endian);
    }

    pub fn write_i64(&mut self, value: i64, endian: Endian) {
        self.write_u64(value as u64, endian);
    }

    pub fn write_f32(&mut self, value: f32, endian: Endian) {
        self.write_u32(value.to_bits(), endian);
    }

    pub fn write_f64(&mut self, value: f64, endian: Endian) {
        self.write_u64(value.to_bits(), endian);
    }

    /// Write the low (little-endian) or high (big-endian) `bytes` of `value`,
    /// as used by `guint-sized` fields.
    pub fn write_uint(&mut self, value: u64, bytes: usize, endian: Endian) -> Result<(), Error> {
        if bytes > 8 {
            return Err(Error::InvalidLength);
        }

        match endian {
            Endian::Little => self.buf.extend_from_slice(&value.to_le_bytes()[..bytes]),
            Endian::Big => self
                .buf
                .extend_from_slice(&value.to_be_bytes()[8 - bytes..]),
        }

        Ok(())
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Write a string, optionally with a length prefix or padded to a fixed size.
    pub fn write_string(
        &mut self,
        value: &str,
        prefix_bytes: u8,
        fixed_size: Option<usize>,
    ) -> Result<(), Error> {
        let bytes = value.as_bytes();

        if let Some(size) = fixed_size {
            if bytes.len() > size {
                return Err(Error::TooLong);
            }

            self.buf.extend_from_slice(bytes);
            self.buf.resize(self.buf.len() + (size - bytes.len()), 0);
            return Ok(());
        }

        match prefix_bytes {
            0 => {}
            1 => {
                let length = u8::try_from(bytes.len()).map_err(|_| Error::TooLong)?;
                self.buf.push(length);
            }
            2 => {
                let length = u16::try_from(bytes.len()).map_err(|_| Error::TooLong)?;
                self.buf.extend_from_slice(&length.to_le_bytes());
            }
            _ => return Err(Error::InvalidLength),
        }

        self.buf.extend_from_slice(bytes);

        Ok(())
    }
}
