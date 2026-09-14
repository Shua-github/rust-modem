use crate::OperationResult;

/// Errors returned while encoding or decoding QMI messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("buffer ended before the value was fully read")]
    Truncated,
    #[error("TLV 0x{id:02x} has trailing bytes")]
    TrailingBytes { id: u8 },
    #[error("mandatory TLV 0x{id:02x} is missing")]
    MissingTlv { id: u8 },
    #[error("invalid length prefix, fixed size or element count")]
    InvalidLength,
    #[error("string field is not valid UTF-8")]
    InvalidString,
    #[error("value does not fit the field")]
    TooLong,
    #[error("QMI protocol error: {0}")]
    Qmi(OperationResult),
}
