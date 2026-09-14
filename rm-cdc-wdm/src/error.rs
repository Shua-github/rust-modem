/// Errors returned by the generic CDC WDM transport.
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    #[error("USB transfer failed: {0:?}")]
    Usb(E),
    #[error("no CDC WDM interface found")]
    NoWdmInterface,
    #[error("invalid USB descriptor")]
    InvalidDescriptor,
    #[error("notification shorter than CDC header")]
    ShortNotification,
    #[error("payload exceeds wMaxCommand")]
    TooLong,
}
