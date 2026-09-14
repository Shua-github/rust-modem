use crate::Error;

pub const NOTIFY_NETWORK_CONNECTION: u8 = 0x00;
pub const NOTIFY_RESPONSE_AVAILABLE: u8 = 0x01;
pub const NOTIFY_SERIAL_STATE: u8 = 0x20;
pub const NOTIFY_SPEED_CHANGE: u8 = 0x2a;

/// A CDC notification (`usb_cdc_notification`) received on the interrupt endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notification {
    pub request_type: u8,
    pub kind: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl Notification {
    pub const SIZE: usize = 8;

    pub fn parse<E>(buffer: &[u8]) -> Result<Self, Error<E>> {
        if buffer.len() < Self::SIZE {
            return Err(Error::ShortNotification);
        }

        Ok(Self {
            request_type: buffer[0],
            kind: buffer[1],
            value: u16::from_le_bytes([buffer[2], buffer[3]]),
            index: u16::from_le_bytes([buffer[4], buffer[5]]),
            length: u16::from_le_bytes([buffer[6], buffer[7]]),
        })
    }
}
