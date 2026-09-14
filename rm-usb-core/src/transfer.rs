use crate::{SetupPacket, TransferType};

pub enum TransferBuffer<'a> {
    In(&'a mut [u8]),
    Out(&'a [u8]),
}

pub struct Transfer<'a> {
    /// Endpoint address: the number in the low nibble, the direction in the
    /// top bit, `0x80` for IN.
    pub endpoint: u8,
    pub transfer_type: TransferType,
    pub buffer: TransferBuffer<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferResult {
    pub bytes_transferred: usize,
}

#[trait_variant::make(UsbTransfer: Send)]
pub trait LocalUsbTransfer {
    type Error;

    async fn control_transfer(
        &mut self,
        setup: SetupPacket,
        buffer: TransferBuffer<'_>,
    ) -> Result<TransferResult, Self::Error>;

    async fn transfer(&mut self, transfer: Transfer<'_>) -> Result<TransferResult, Self::Error>;

    /// Whether `error` reports a USB STALL, i.e. a halted endpoint.
    fn is_stall(error: &Self::Error) -> bool;

    /// Clear the halt on the bulk or interrupt endpoint `endpoint`.
    ///
    /// A halted endpoint rejects every later transfer until the host sends
    /// `CLEAR_FEATURE(ENDPOINT_HALT)`, which is what this does.
    async fn clear_halt(
        &mut self,
        transfer_type: TransferType,
        endpoint: u8,
    ) -> Result<(), Self::Error>;
}
