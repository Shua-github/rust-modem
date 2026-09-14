use crate::{DirectionType, LocalUsbDevice, TransferType};
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFilter {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub class: Option<ClassFilter>,
    pub interface: Option<InterfaceFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassFilter {
    pub class: Option<u8>,
    pub subclass: Option<u8>,
    pub protocol: Option<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceFilter {
    pub number: Option<u8>,
    pub class: Option<ClassFilter>,
    pub endpoint: Option<EndpointFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointFilter {
    pub address: Option<u8>,
    pub direction: Option<DirectionType>,
    pub transfer_type: Option<TransferType>,
}

impl DeviceFilter {
    pub const fn new(vendor_id: u16, product_id: u16) -> Self {
        Self {
            vendor_id: Some(vendor_id),
            product_id: Some(product_id),
            class: None,
            interface: None,
        }
    }

    pub const fn class(class: u8, subclass: u8, protocol: u8) -> Self {
        Self {
            vendor_id: None,
            product_id: None,
            class: Some(ClassFilter {
                class: Some(class),
                subclass: Some(subclass),
                protocol: Some(protocol),
            }),
            interface: None,
        }
    }

    pub const fn any() -> Self {
        Self {
            vendor_id: None,
            product_id: None,
            class: None,
            interface: None,
        }
    }
}

#[trait_variant::make(UsbBus: Send)]
pub trait LocalUsbBus {
    type Error;
    type Device: LocalUsbDevice<Error = Self::Error>;

    /// Return devices already available to the backend without prompting.
    ///
    /// On WebUSB this is `navigator.usb.getDevices()` and therefore only
    /// contains devices previously authorized by the user.
    async fn devices(&mut self) -> Result<Vec<Self::Device>, Self::Error>;

    /// Request the user/backend to provide a USB device.
    ///
    /// On WebUSB this corresponds to `navigator.usb.requestDevice()`.
    /// On native platforms this may enumerate devices and select one.
    async fn request_device(
        &mut self,
        filters: &[DeviceFilter],
    ) -> Result<Self::Device, Self::Error>;
}
