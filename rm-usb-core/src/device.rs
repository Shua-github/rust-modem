use alloc::vec::Vec;

use crate::LocalUsbTransfer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceInfo {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub product_id: u16,
    pub vendor_id: u16,
    pub usb_version: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub interfaces: Vec<InterfaceInfo>,
}

#[trait_variant::make(UsbDevice: Send)]
pub trait LocalUsbDevice {
    type Error;
    // Bound on the local (non-Send) trait so single-threaded backends such as
    // WebUSB can implement this; the `UsbDevice` variant adds Send itself.
    type Transfer: LocalUsbTransfer<Error = Self::Error>;

    fn info(&self) -> DeviceInfo;

    async fn open(&mut self) -> Result<(), Self::Error>;

    async fn close(&mut self) -> Result<(), Self::Error>;

    async fn select_configuration(&mut self, configuration: u8) -> Result<(), Self::Error>;

    async fn claim_interface(&mut self, interface: u8) -> Result<(), Self::Error>;

    async fn release_interface(&mut self, interface: u8) -> Result<(), Self::Error>;

    /// Select the alternate setting of `interface`, which must be claimed.
    ///
    /// CDC functions keep their bulk data path on a paired interface and only
    /// start answering once that interface's data altsetting is selected, which
    /// the kernel's own drivers do on bind.
    async fn set_alternate_setting(
        &mut self,
        interface: u8,
        alternate_setting: u8,
    ) -> Result<(), Self::Error>;

    async fn reset(&mut self) -> Result<(), Self::Error>;

    fn transfer(&mut self) -> Self::Transfer;
}
