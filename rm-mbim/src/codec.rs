use alloc::{string::String, vec::Vec};

use crate::Error;

/// Encoder for an MBIM information buffer.
///
/// Fixed fields are emitted first. Reference fields are collected and their
/// offsets are patched when [`Encoder::finish`] is called.
#[derive(Debug, Default)]
pub struct Encoder {
    fixed: Vec<u8>,
    data: Vec<u8>,
    references: Vec<(usize, usize)>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write_u16(&mut self, value: u16) {
        self.fixed.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u32(&mut self, value: u32) {
        self.fixed.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i32(&mut self, value: i32) {
        self.fixed.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u64(&mut self, value: u64) {
        self.fixed.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_bytes(&mut self, value: &[u8]) {
        self.fixed.extend_from_slice(value);
    }

    pub fn write_ref(&mut self, value: &[u8]) {
        if value.is_empty() {
            self.fixed.extend_from_slice(&[0u8; 8]);
            return;
        }
        let position = self.fixed.len();
        self.fixed.extend_from_slice(&0u32.to_le_bytes());
        self.fixed
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        let data_position = self.data.len();
        self.data.extend_from_slice(value);
        pad4(&mut self.data);
        self.references.push((position, data_position));
    }

    /// A UICC reference byte array: unlike every other reference these fields
    /// carry the length first and the offset second (libmbim's
    /// `swapped_offset_length`), which the EM05-CE relies on for its ATR.
    pub fn write_ref_swapped(&mut self, value: &[u8]) {
        if value.is_empty() {
            self.fixed.extend_from_slice(&[0u8; 8]);
            return;
        }
        let position = self.fixed.len();
        self.fixed
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.fixed.extend_from_slice(&0u32.to_le_bytes());
        let data_position = self.data.len();
        self.data.extend_from_slice(value);
        pad4(&mut self.data);
        // Only the offset (the second word) needs patching.
        self.references.push((position + 4, data_position));
    }

    pub fn write_string(&mut self, value: &str, utf8: bool) {
        if utf8 {
            self.write_ref(value.as_bytes());
            return;
        }

        let mut bytes = Vec::with_capacity((value.encode_utf16().count() + 1) * 2);
        for unit in value.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
        self.write_ref(&bytes);
    }

    pub fn write_string_array(&mut self, values: &[String], utf8: bool) {
        for value in values {
            self.write_string(value, utf8);
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        let data_offset = self.fixed.len();
        for (position, data_position) in self.references {
            let offset = (data_offset + data_position) as u32;
            self.fixed[position..position + 4].copy_from_slice(&offset.to_le_bytes());
        }
        self.fixed.extend_from_slice(&self.data);
        self.fixed
    }
}

/// Decoder for an MBIM information buffer.
#[derive(Debug, Clone, Copy)]
pub struct Decoder<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub fn read_u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(*self.take::<2>()?))
    }

    pub fn read_u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(*self.take::<4>()?))
    }

    pub fn read_i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(*self.take::<4>()?))
    }

    pub fn read_u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(*self.take::<8>()?))
    }

    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let start = self.position;
        let end = start.checked_add(length).ok_or(Error::InvalidLength)?;
        let value = self.data.get(start..end).ok_or(Error::Truncated)?;
        self.position = end;
        Ok(value)
    }

    pub fn read_ref(&mut self) -> Result<&'a [u8], Error> {
        let offset = self.read_u32()? as usize;
        let length = self.read_u32()? as usize;
        let end = offset.checked_add(length).ok_or(Error::InvalidLength)?;
        self.data.get(offset..end).ok_or(Error::InvalidLength)
    }

    /// Reads a UICC reference byte array, whose length comes before its offset.
    pub fn read_ref_swapped(&mut self) -> Result<&'a [u8], Error> {
        let length = self.read_u32()? as usize;
        let offset = self.read_u32()? as usize;
        let end = offset.checked_add(length).ok_or(Error::InvalidLength)?;
        self.data.get(offset..end).ok_or(Error::InvalidLength)
    }

    pub fn read_string(&mut self, utf8: bool) -> Result<String, Error> {
        let bytes = self.read_ref()?;
        if utf8 {
            return String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidString);
        }
        if !bytes.len().is_multiple_of(2) {
            return Err(Error::InvalidLength);
        }
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
        let mut units = units.collect::<Vec<_>>();
        while units.last() == Some(&0) {
            units.pop();
        }
        String::from_utf16(&units).map_err(|_| Error::InvalidString)
    }

    pub fn read_remaining(&mut self) -> Result<&'a [u8], Error> {
        let value = self.data.get(self.position..).ok_or(Error::Truncated)?;
        self.position = self.data.len();
        Ok(value)
    }

    pub fn subdecoder(&self, data: &'a [u8]) -> Self {
        Self::new(data)
    }

    fn take<const N: usize>(&mut self) -> Result<&'a [u8; N], Error> {
        let bytes = self.read_bytes(N)?;
        bytes.try_into().map_err(|_| Error::InvalidLength)
    }
}

fn pad4(data: &mut Vec<u8>) {
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
}
