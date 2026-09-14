use alloc::{vec, vec::Vec};

use crate::Error;

/// MBIM message type values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MessageType {
    Open = 0x0000_0001,
    Close = 0x0000_0002,
    Command = 0x0000_0003,
    HostError = 0x0000_0004,
    OpenDone = 0x8000_0001,
    CloseDone = 0x8000_0002,
    CommandDone = 0x8000_0003,
    FunctionError = 0x8000_0004,
    IndicateStatus = 0x8000_0007,
}

impl MessageType {
    fn from_raw(value: u32) -> Result<Self, Error> {
        match value {
            0x0000_0001 => Ok(Self::Open),
            0x0000_0002 => Ok(Self::Close),
            0x0000_0003 => Ok(Self::Command),
            0x0000_0004 => Ok(Self::HostError),
            0x8000_0001 => Ok(Self::OpenDone),
            0x8000_0002 => Ok(Self::CloseDone),
            0x8000_0003 => Ok(Self::CommandDone),
            0x8000_0004 => Ok(Self::FunctionError),
            0x8000_0007 => Ok(Self::IndicateStatus),
            value => Err(Error::InvalidMessageType(value)),
        }
    }

    pub const fn is_fragmented(self) -> bool {
        matches!(
            self,
            Self::Command | Self::CommandDone | Self::IndicateStatus
        )
    }
}

/// MBIM query/set command type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CommandType {
    Query = 0,
    Set = 1,
}

impl CommandType {
    fn from_raw(value: u32) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Query),
            1 => Ok(Self::Set),
            value => Err(Error::InvalidCommandType(value)),
        }
    }
}

/// A MBIM UUID in the byte order used on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Uuid(pub [u8; 16]);

impl Uuid {
    /// UUID of the standard Basic Connect service.
    pub const BASIC_CONNECT: Self = Self([
        0xa2, 0x89, 0xcc, 0x33, 0xbc, 0xbb, 0x8b, 0x4f, 0xb6, 0xb0, 0x13, 0x3e, 0xc2, 0xaa, 0xe6,
        0xdf,
    ]);

    /// Parse the usual 36-character UUID spelling.
    pub fn parse(value: &str) -> Result<Self, Error> {
        let mut bytes = [0u8; 16];
        let mut index = 0;
        let mut high = None;

        for byte in value.bytes() {
            if byte == b'-' {
                continue;
            }

            let nibble = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(Error::InvalidLength),
            };

            if let Some(previous) = high.take() {
                if index >= bytes.len() {
                    return Err(Error::InvalidLength);
                }
                bytes[index] = (previous << 4) | nibble;
                index += 1;
            } else {
                high = Some(nibble);
            }
        }

        if high.is_some() || index != bytes.len() {
            return Err(Error::InvalidLength);
        }

        Ok(Self(bytes))
    }
}

impl core::fmt::Display for Uuid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let b = self.0;

        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15],
        )
    }
}

/// Parsed fields shared by command and command-done messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandInfo<'a> {
    pub service: Uuid,
    pub cid: u32,
    pub command_type: Option<CommandType>,
    pub status: Option<u32>,
    pub information_buffer: &'a [u8],
}

/// A validated MBIM message without its USB transport framing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub message_type: MessageType,
    pub transaction_id: u32,
    payload: Vec<u8>,
}

impl Message {
    pub fn open(transaction_id: u32, max_control_transfer: u32) -> Self {
        Self::with_payload(
            MessageType::Open,
            transaction_id,
            max_control_transfer.to_le_bytes().to_vec(),
        )
    }

    pub fn close(transaction_id: u32) -> Self {
        Self::with_payload(MessageType::Close, transaction_id, Vec::new())
    }

    pub fn command(
        transaction_id: u32,
        service: Uuid,
        cid: u32,
        command_type: CommandType,
        information_buffer: &[u8],
    ) -> Self {
        let mut payload = Vec::with_capacity(44 + information_buffer.len());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&service.0);
        payload.extend_from_slice(&cid.to_le_bytes());
        payload.extend_from_slice(&(command_type as u32).to_le_bytes());
        payload.extend_from_slice(&(information_buffer.len() as u32).to_le_bytes());
        payload.extend_from_slice(information_buffer);

        Self::with_payload(MessageType::Command, transaction_id, payload)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 12 {
            return Err(Error::Truncated);
        }

        let message_type =
            MessageType::from_raw(u32::from_le_bytes(bytes[0..4].try_into().unwrap()))?;
        let length = usize::try_from(u32::from_le_bytes(bytes[4..8].try_into().unwrap()))
            .map_err(|_| Error::InvalidLength)?;
        if length < 12 || bytes.len() < length {
            return Err(Error::InvalidLength);
        }

        let transaction_id = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let payload = bytes[12..length].to_vec();

        if message_type.is_fragmented() {
            if payload.len() < 8 {
                return Err(Error::Truncated);
            }
            let total = u32::from_le_bytes(payload[0..4].try_into().unwrap());
            let current = u32::from_le_bytes(payload[4..8].try_into().unwrap());
            if total == 0 || current >= total {
                return Err(Error::InvalidFragment);
            }
        }

        Ok(Self {
            message_type,
            transaction_id,
            payload,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let length = 12 + self.payload.len();
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(&(self.message_type as u32).to_le_bytes());
        bytes.extend_from_slice(&(length as u32).to_le_bytes());
        bytes.extend_from_slice(&self.transaction_id.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    pub fn fragment(&self) -> Result<(u32, u32), Error> {
        if !self.message_type.is_fragmented() || self.payload.len() < 8 {
            return Err(Error::InvalidFragment);
        }
        Ok((
            u32::from_le_bytes(self.payload[0..4].try_into().unwrap()),
            u32::from_le_bytes(self.payload[4..8].try_into().unwrap()),
        ))
    }

    pub fn fragment_payload(&self) -> Result<&[u8], Error> {
        self.fragment()?;
        Ok(&self.payload[8..])
    }

    pub fn command_info(&self) -> Result<CommandInfo<'_>, Error> {
        if !matches!(
            self.message_type,
            MessageType::Command | MessageType::CommandDone
        ) {
            return Err(Error::UnexpectedResponse);
        }
        if self.payload.len() < 8 + 16 + 12 {
            return Err(Error::Truncated);
        }

        let service = Uuid(self.payload[8..24].try_into().unwrap());
        let cid = u32::from_le_bytes(self.payload[24..28].try_into().unwrap());
        let value = u32::from_le_bytes(self.payload[28..32].try_into().unwrap());
        let length = usize::try_from(u32::from_le_bytes(self.payload[32..36].try_into().unwrap()))
            .map_err(|_| Error::InvalidLength)?;
        let end = 36usize.checked_add(length).ok_or(Error::InvalidLength)?;
        if end > self.payload.len() {
            return Err(Error::InvalidLength);
        }

        Ok(CommandInfo {
            service,
            cid,
            command_type: (self.message_type == MessageType::Command)
                .then(|| CommandType::from_raw(value))
                .transpose()?,
            status: (self.message_type == MessageType::CommandDone).then_some(value),
            information_buffer: &self.payload[36..end],
        })
    }

    pub fn status_code(&self) -> Result<u32, Error> {
        match self.message_type {
            MessageType::OpenDone | MessageType::CloseDone | MessageType::FunctionError => {
                if self.payload.len() < 4 {
                    return Err(Error::Truncated);
                }
                Ok(u32::from_le_bytes(self.payload[0..4].try_into().unwrap()))
            }
            MessageType::CommandDone => Ok(self.command_info()?.status.unwrap()),
            _ => Err(Error::UnexpectedResponse),
        }
    }

    /// Split a command-like message into MBIM fragments no larger than
    /// `max_size`. Non-fragmentable messages must already fit.
    pub fn fragments(&self, max_size: u16) -> Result<Vec<Vec<u8>>, Error> {
        let bytes = self.to_bytes();
        let max_size = usize::from(max_size);
        if bytes.len() <= max_size {
            return Ok(vec![bytes]);
        }
        if !self.message_type.is_fragmented() || max_size < 20 {
            return Err(Error::TooLong);
        }

        let fragment_payload_size = max_size - 20;
        let data = &bytes[20..];
        let total = data.len().div_ceil(fragment_payload_size) as u32;
        let mut fragments = Vec::with_capacity(total as usize);

        for (current, chunk) in data.chunks(fragment_payload_size).enumerate() {
            let length = 20 + chunk.len();
            let mut fragment = Vec::with_capacity(length);
            fragment.extend_from_slice(&(self.message_type as u32).to_le_bytes());
            fragment.extend_from_slice(&(length as u32).to_le_bytes());
            fragment.extend_from_slice(&self.transaction_id.to_le_bytes());
            fragment.extend_from_slice(&total.to_le_bytes());
            fragment.extend_from_slice(&(current as u32).to_le_bytes());
            fragment.extend_from_slice(chunk);
            fragments.push(fragment);
        }

        Ok(fragments)
    }

    pub fn merge_fragments(fragments: &[Self]) -> Result<Self, Error> {
        let first = fragments.first().ok_or(Error::InvalidFragment)?;
        let (total, current) = first.fragment()?;
        if current != 0 || total as usize != fragments.len() {
            return Err(Error::InvalidFragment);
        }

        let mut payload = Vec::new();
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (index, fragment) in fragments.iter().enumerate() {
            let (fragment_total, fragment_current) = fragment.fragment()?;
            if fragment.message_type != first.message_type
                || fragment.transaction_id != first.transaction_id
                || fragment_total != total
                || fragment_current != index as u32
            {
                return Err(Error::InvalidFragment);
            }
            payload.extend_from_slice(fragment.fragment_payload()?);
        }

        Ok(Self::with_payload(
            first.message_type,
            first.transaction_id,
            payload,
        ))
    }

    fn with_payload(message_type: MessageType, transaction_id: u32, payload: Vec<u8>) -> Self {
        Self {
            message_type,
            transaction_id,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trip() {
        let message = Message::command(
            7,
            Uuid::BASIC_CONNECT,
            3,
            CommandType::Query,
            &[0x01, 0x02, 0x03],
        );
        let decoded = Message::from_bytes(&message.to_bytes()).unwrap();
        let info = decoded.command_info().unwrap();

        assert_eq!(decoded.transaction_id, 7);
        assert_eq!(info.service, Uuid::BASIC_CONNECT);
        assert_eq!(info.cid, 3);
        assert_eq!(info.command_type, Some(CommandType::Query));
        assert_eq!(info.information_buffer, &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn fragments_round_trip() {
        let data: Vec<u8> = (0..100).collect();
        let message = Message::command(9, Uuid::BASIC_CONNECT, 1, CommandType::Set, &data);
        let encoded = message.fragments(32).unwrap();
        let fragments: Vec<_> = encoded
            .iter()
            .map(|fragment| Message::from_bytes(fragment).unwrap())
            .collect();
        let merged = Message::merge_fragments(&fragments).unwrap();

        assert_eq!(merged.command_info().unwrap().information_buffer, data);
    }

    #[test]
    fn uuid_round_trip() {
        let uuid = Uuid::parse("a289cc33-bcbb-8b4f-b6b0-133ec2aae6df").unwrap();
        assert_eq!(uuid, Uuid::BASIC_CONNECT);
        assert_eq!(
            alloc::format!("{}", uuid),
            "a289cc33-bcbb-8b4f-b6b0-133ec2aae6df"
        );
    }
}
