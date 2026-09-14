/// Errors returned by the MBIM codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("MBIM message is truncated")]
    Truncated,
    #[error("invalid MBIM message length")]
    InvalidLength,
    #[error("invalid MBIM message type 0x{0:08x}")]
    InvalidMessageType(u32),
    #[error("invalid MBIM message fragment")]
    InvalidFragment,
    #[error("invalid MBIM command type {0}")]
    InvalidCommandType(u32),
    #[error("MBIM message exceeds the device control-message limit")]
    TooLong,
    #[error("MBIM protocol error 0x{0:08x}")]
    Status(u32),
    #[error("unexpected MBIM response")]
    UnexpectedResponse,
    #[error("invalid MBIM string")]
    InvalidString,
}
