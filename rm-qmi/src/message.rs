use alloc::vec::Vec;

use crate::{Endian, Error, Tlv, TlvBuf, TlvWriter, tlvs};

/// QMUX transport marker, used by services that fit in 8 bits.
pub const QMUX_MARKER: u8 = 0x01;
/// QRTR transport marker, used by services with a 16-bit service id.
pub const QRTR_MARKER: u8 = 0x02;

/// Bytes in front of a QMI message: the marker and the QMUX/QRTR header.
pub const FRAME_HEADER_LEN: usize = 6;

/// Well-known QMI service ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum Service {
    Ctl = 0x00,
    Wds = 0x01,
    Dms = 0x02,
    Nas = 0x03,
    Qos = 0x04,
    Wms = 0x05,
    Pds = 0x06,
    Auth = 0x07,
    At = 0x08,
    Voice = 0x09,
    Uim = 0x0b,
    Pbm = 0x0c,
    Loc = 0x10,
    Sar = 0x11,
    Ims = 0x12,
    Wda = 0x1a,
    Imsp = 0x1f,
    Imsa = 0x21,
    Pdc = 0x24,
    Dsd = 0x2a,
    Dpm = 0x2f,
    Oma = 0xe2,
    Fox = 0xe3,
    Gms = 0xe7,
    Gas = 0xe8,
    Atr = 0xed,
    Ssc = 0x190,
    Imsdcm = 0x302,
}

/// A decoded QMI message: header fields plus the TLV list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub service: u16,
    pub client_id: u8,
    pub transaction_id: u16,
    /// QMI message flags (the first header byte).
    pub flags: u8,
    pub message_id: u16,
    pub tlvs: Vec<TlvBuf>,
}

impl Message {
    pub fn new(service: u16, client_id: u8, transaction_id: u16, message_id: u16) -> Self {
        Self {
            service,
            client_id,
            transaction_id,
            flags: 0,
            message_id,
            tlvs: Vec::new(),
        }
    }

    pub fn with_tlvs(mut self, tlvs: Vec<TlvBuf>) -> Self {
        self.tlvs = tlvs;
        self
    }

    pub fn is_control(&self) -> bool {
        self.service == Service::Ctl as u16
    }

    pub fn iter_tlvs(&self) -> impl Iterator<Item = Tlv<'_>> {
        self.tlvs.iter().map(|tlv| Tlv {
            id: tlv.id,
            value: &tlv.value,
        })
    }

    pub fn find_tlv(&self, id: u8) -> Option<Tlv<'_>> {
        self.iter_tlvs().find(|tlv| tlv.id == id)
    }

    /// Encode the QMI header and TLVs, without any transport framing.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut body = TlvWriter::new();

        for tlv in &self.tlvs {
            body.write_u8(tlv.id);
            body.write_u16(tlv.value.len() as u16, Endian::Little);
            body.write_bytes(&tlv.value);
        }

        let mut out = Vec::with_capacity(body.len() + 7);
        out.push(self.flags);

        if self.is_control() {
            out.push(self.transaction_id as u8);
        } else {
            out.extend_from_slice(&self.transaction_id.to_le_bytes());
        }

        out.extend_from_slice(&self.message_id.to_le_bytes());
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(body.as_slice());

        out
    }

    /// Decode a QMI header and TLV list, without any transport framing.
    ///
    /// The service id is not part of the QMI body, so it must be supplied by
    /// the caller (for example from [`Self::from_qmux_bytes`]).
    pub fn from_bytes(service: u16, bytes: &[u8]) -> Result<Self, Error> {
        let header_len = if service == Service::Ctl as u16 { 6 } else { 7 };

        if bytes.len() < header_len {
            return Err(Error::Truncated);
        }

        let flags = bytes[0];

        let (transaction_id, offset) = if header_len == 6 {
            (u16::from(bytes[1]), 2)
        } else {
            (u16::from_le_bytes([bytes[1], bytes[2]]), 3)
        };

        let message_id = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        let tlv_length = usize::from(u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]));
        let body = &bytes[header_len..];

        if body.len() < tlv_length {
            return Err(Error::Truncated);
        }

        let mut list = Vec::new();

        for tlv in tlvs(&body[..tlv_length]) {
            list.push(TlvBuf {
                id: tlv.id,
                value: tlv.value.to_vec(),
            });
        }

        Ok(Self {
            service,
            client_id: 0,
            transaction_id,
            flags,
            message_id,
            tlvs: list,
        })
    }

    /// Encode a full transport frame: marker, QMUX/QRTR header and message.
    pub fn to_qmux_bytes(&self) -> Vec<u8> {
        let body = self.to_bytes();
        let mut out = Vec::with_capacity(body.len() + FRAME_HEADER_LEN);
        out.extend_from_slice(&Self::frame_header(
            self.service,
            self.client_id,
            body.len(),
        ));
        out.extend_from_slice(&body);

        out
    }

    /// The marker and transport header a frame of `body_len` bytes carries.
    ///
    /// The length field covers this header and the message, but not the
    /// marker, and the service is written as eight bits whenever it fits in
    /// them: a service with a 16-bit id is the only thing the second marker
    /// exists for.
    pub fn frame_header(service: u16, client_id: u8, body_len: usize) -> [u8; FRAME_HEADER_LEN] {
        let length = (body_len + FRAME_HEADER_LEN - 1) as u16;
        let mut header = [0u8; FRAME_HEADER_LEN];

        if service <= u16::from(u8::MAX) {
            header[0] = QMUX_MARKER;
            header[1..3].copy_from_slice(&length.to_le_bytes());
            header[3] = 0; // transport flags
            header[4] = service as u8;
            header[5] = client_id;
        } else {
            header[0] = QRTR_MARKER;
            header[1..3].copy_from_slice(&length.to_le_bytes());
            header[3..5].copy_from_slice(&service.to_le_bytes());
            header[5] = client_id;
        }

        header
    }

    /// Decode a full transport frame.
    pub fn from_qmux_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let marker = *bytes.first().ok_or(Error::Truncated)?;

        let (service, client_id) = match marker {
            QMUX_MARKER => {
                if bytes.len() < 6 {
                    return Err(Error::Truncated);
                }
                (u16::from(bytes[4]), bytes[5])
            }
            QRTR_MARKER => {
                if bytes.len() < 6 {
                    return Err(Error::Truncated);
                }
                (u16::from_le_bytes([bytes[3], bytes[4]]), bytes[5])
            }
            _ => return Err(Error::InvalidLength),
        };

        // Marker plus the 5-byte QMUX/QRTR header.
        let mut message = Self::from_bytes(service, &bytes[6..])?;
        message.client_id = client_id;

        Ok(message)
    }
}
