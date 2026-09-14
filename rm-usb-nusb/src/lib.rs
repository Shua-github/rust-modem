//! nusb-backed implementation of the [`rm_usb_core`] traits.
//!
//! On the desktop platforms nusb enumerates and opens the device itself, and
//! [`NusbBus`] hands those out. Android has neither half of that: its USB host
//! API is the only door to a device, and what it hands back is an open
//! descriptor. [`NusbDevice::from_fd`] wraps that descriptor and everything
//! after it is the same, which is the whole backend on that platform.

#[cfg(unix)]
use std::os::fd::OwnedFd;
use std::time::Duration;

use futures::future::Either;
use nusb::transfer::{
    Buffer, Bulk, BulkOrInterrupt, Completion, ControlIn, ControlOut,
    ControlType as NusbControlType, EndpointDirection, In, Interrupt, Out,
    Recipient as NusbRecipient, TransferError,
};
use nusb::{Device, Endpoint, GetDescriptorError, Interface, InterfaceInfo as NusbInterfaceInfo};

#[cfg(not(target_os = "android"))]
use rm_usb_core::{ClassFilter, DeviceFilter, UsbBus};
use rm_usb_core::{
    ControlType, DeviceInfo, InterfaceInfo, Recipient, SetupPacket, Transfer, TransferBuffer,
    TransferResult, TransferType, UsbDevice, UsbTransfer,
};

/// Timeout applied to every USB transfer.
const TIMEOUT: Duration = Duration::from_secs(5);

const GET_DESCRIPTOR: u8 = 0x06;

/// Errors produced by the nusb backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Device enumeration, open or interface claim failed.
    #[error("usb error: {0}")]
    Nusb(#[source] nusb::Error),
    /// A data transfer failed.
    #[error("transfer error: {0}")]
    Transfer(#[source] TransferError),
    /// The native descriptor reader failed.
    #[error("get descriptor error: {0}")]
    GetDescriptor(#[source] GetDescriptorError),
    /// No device matched the requested filter.
    #[error("no matching usb device")]
    NoDevice,
    /// The device is not open (or the interface is not claimed).
    #[error("usb device is not open")]
    NotOpen,
    /// The request cannot be expressed with the nusb API.
    #[error("request not supported by nusb")]
    Unsupported,
}

/// USB bus backed by nusb's device enumeration.
///
/// Android has no enumeration path in nusb: a device there is only ever
/// obtained from an already-open descriptor, so there is nothing for a bus to
/// list. [`NusbDevice::from_fd`] is the whole story on that platform.
#[cfg(not(target_os = "android"))]
pub struct NusbBus {
    devices: Vec<NusbDevice>,
}

#[cfg(not(target_os = "android"))]
impl NusbBus {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    async fn enumerate(&mut self) -> Result<(), Error> {
        self.devices = nusb::list_devices()
            .await
            .map_err(Error::Nusb)?
            .map(NusbDevice::from_info)
            .collect();

        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
impl Default for NusbBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_os = "android"))]
impl UsbBus for NusbBus {
    type Error = Error;
    type Device = NusbDevice;

    async fn devices(&mut self) -> Result<Vec<NusbDevice>, Error> {
        self.enumerate().await?;
        Ok(self.devices.clone())
    }

    async fn request_device(&mut self, filters: &[DeviceFilter]) -> Result<NusbDevice, Error> {
        self.enumerate().await?;

        self.devices
            .iter()
            .find(|device| filters.iter().any(|filter| device.matches(filter)))
            .cloned()
            .ok_or(Error::NoDevice)
    }
}

/// An enumerated USB device. Opening and claiming are lazy.
///
/// A device is either one `nusb` found by enumeration, or one wrapped around a
/// descriptor that somebody else opened.
#[derive(Clone)]
pub struct NusbDevice {
    source: Source,
    device: Option<Device>,
    /// Interfaces claimed so far. The first is the function's own interface,
    /// which routes the control transfers; extra claims, such as a CDC data
    /// interface, are kept alive alongside it.
    interfaces: Vec<Interface>,
}

/// Where a [`NusbDevice`] came from, and therefore how it opens.
#[derive(Clone)]
enum Source {
    /// A device `nusb` enumerated, opened lazily on [`UsbDevice::open`].
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Enumerated(Box<nusb::DeviceInfo>),
    /// A device wrapped around a usbfs descriptor that is already open, such as
    /// the one Android's `UsbDeviceConnection.getFileDescriptor()` hands out.
    /// Such a device carries its own summary, because there is no enumeration
    /// record to read it from.
    #[cfg_attr(not(unix), allow(dead_code))]
    Descriptor(DeviceInfo),
}

impl NusbDevice {
    #[cfg(not(target_os = "android"))]
    fn from_info(info: nusb::DeviceInfo) -> Self {
        Self {
            source: Source::Enumerated(Box::new(info)),
            device: None,
            interfaces: Vec::new(),
        }
    }

    /// Wrap a usbfs descriptor that is already open.
    ///
    /// Android has neither the enumeration nor the open-by-id path `nusb` uses
    /// elsewhere: its USB host API opens the device, and what it hands out is a
    /// file descriptor. Everything after that is the same, so the device is
    /// described by the caller, who can read the descriptor through the
    /// platform API.
    #[cfg(unix)]
    pub async fn from_fd(fd: OwnedFd, info: DeviceInfo) -> Result<Self, Error> {
        let device = nusb::Device::from_fd(fd).await.map_err(Error::Nusb)?;

        Ok(Self {
            source: Source::Descriptor(info),
            device: Some(device),
            interfaces: Vec::new(),
        })
    }

    /// The nusb enumeration record, which exposes interface descriptors
    /// without opening the device. `None` for a device wrapped around a
    /// descriptor that somebody else opened.
    pub fn nusb_info(&self) -> Option<&nusb::DeviceInfo> {
        match &self.source {
            Source::Enumerated(info) => Some(info.as_ref()),
            Source::Descriptor(_) => None,
        }
    }

    /// Whether this device exposes an interface (or a device-level class) that
    /// satisfies `filter`.
    #[cfg(not(target_os = "android"))]
    fn matches(&self, filter: &DeviceFilter) -> bool {
        let info = self.info();

        if filter.vendor_id.is_some_and(|id| id != info.vendor_id)
            || filter.product_id.is_some_and(|id| id != info.product_id)
        {
            return false;
        }

        match &filter.class {
            Some(class) => {
                info.interfaces.iter().any(|interface| {
                    class_matches(
                        class,
                        interface.class,
                        interface.subclass,
                        interface.protocol,
                    )
                }) || class_matches(class, info.class, info.subclass, info.protocol)
            }
            None => true,
        }
    }
}

/// Whether `class` accepts the given class/subclass/protocol triple. Unset
/// fields match anything.
#[cfg(not(target_os = "android"))]
fn class_matches(class: &ClassFilter, class_code: u8, subclass: u8, protocol: u8) -> bool {
    class.class.is_none_or(|value| value == class_code)
        && class.subclass.is_none_or(|value| value == subclass)
        && class.protocol.is_none_or(|value| value == protocol)
}

fn to_interface_info(interface: &NusbInterfaceInfo) -> InterfaceInfo {
    InterfaceInfo {
        number: interface.interface_number(),
        class: interface.class(),
        subclass: interface.subclass(),
        protocol: interface.protocol(),
    }
}

impl UsbDevice for NusbDevice {
    type Error = Error;
    type Transfer = NusbTransfer;

    fn info(&self) -> DeviceInfo {
        match &self.source {
            Source::Enumerated(info) => DeviceInfo {
                product_id: info.product_id(),
                vendor_id: info.vendor_id(),
                usb_version: info.usb_version(),
                class: info.class(),
                subclass: info.subclass(),
                protocol: info.protocol(),
                interfaces: info.interfaces().map(to_interface_info).collect(),
            },
            // A wrapped descriptor has no enumeration record, so the caller
            // that opened it is the one that describes it.
            Source::Descriptor(info) => info.clone(),
        }
    }

    async fn open(&mut self) -> Result<(), Error> {
        if self.device.is_none() {
            let opened = match &self.source {
                Source::Enumerated(info) => Some(info.open().await.map_err(Error::Nusb)?),
                // `from_fd` opened the device when it wrapped the descriptor.
                Source::Descriptor(_) => None,
            };

            if let Some(device) = opened {
                self.device = Some(device);
            }
        }

        Ok(())
    }

    async fn close(&mut self) -> Result<(), Error> {
        self.interfaces.clear();
        self.device = None;

        Ok(())
    }

    async fn select_configuration(&mut self, configuration: u8) -> Result<(), Error> {
        let device = self.device.as_ref().ok_or(Error::NotOpen)?;

        match device.set_configuration(configuration).await {
            Ok(()) => Ok(()),
            // WinUSB rejects SET_CONFIGURATION; the OS has already
            // configured the device by the time it is opened.
            Err(error) if matches!(error.kind(), nusb::ErrorKind::Unsupported) => Ok(()),
            Err(error) => Err(Error::Nusb(error)),
        }
    }

    async fn claim_interface(&mut self, interface: u8) -> Result<(), Error> {
        let device = self.device.as_ref().ok_or(Error::NotOpen)?;
        let claimed = device
            .claim_interface(interface)
            .await
            .map_err(Error::Nusb)?;

        self.interfaces
            .retain(|held| held.interface_number() != interface);
        self.interfaces.push(claimed);

        Ok(())
    }

    async fn release_interface(&mut self, interface: u8) -> Result<(), Error> {
        self.interfaces
            .retain(|held| held.interface_number() != interface);

        Ok(())
    }

    async fn set_alternate_setting(
        &mut self,
        interface: u8,
        alternate_setting: u8,
    ) -> Result<(), Error> {
        let claimed = match self
            .interfaces
            .iter()
            .find(|held| held.interface_number() == interface)
        {
            Some(held) => held.clone(),
            None => {
                let device = self.device.as_ref().ok_or(Error::NotOpen)?;
                let claimed = device
                    .claim_interface(interface)
                    .await
                    .map_err(Error::Nusb)?;
                self.interfaces.push(claimed.clone());
                claimed
            }
        };

        claimed
            .set_alt_setting(alternate_setting)
            .await
            .map_err(Error::Nusb)
    }

    async fn reset(&mut self) -> Result<(), Error> {
        let device = self.device.as_ref().ok_or(Error::NotOpen)?;

        device.reset().await.map_err(Error::Nusb)
    }

    fn transfer(&mut self) -> NusbTransfer {
        NusbTransfer {
            device: self.device.clone(),
            interface: self.interfaces.first().cloned(),
        }
    }
}

/// A single transfer issued against an open [`NusbDevice`].
pub struct NusbTransfer {
    device: Option<Device>,
    interface: Option<Interface>,
}

impl UsbTransfer for NusbTransfer {
    type Error = Error;

    async fn control_transfer(
        &mut self,
        setup: SetupPacket,
        buffer: TransferBuffer<'_>,
    ) -> Result<TransferResult, Error> {
        if let Some(interface) = &self.interface {
            let control_type = match setup.control_type {
                ControlType::Standard => NusbControlType::Standard,
                ControlType::Class => NusbControlType::Class,
                ControlType::Vendor => NusbControlType::Vendor,
            };
            let recipient = match setup.recipient {
                Recipient::Device => NusbRecipient::Device,
                Recipient::Interface => NusbRecipient::Interface,
                Recipient::Endpoint => NusbRecipient::Endpoint,
                Recipient::Other => NusbRecipient::Other,
            };

            return match buffer {
                TransferBuffer::In(out) => {
                    let data = interface
                        .control_in(
                            ControlIn {
                                control_type,
                                recipient,
                                request: setup.request,
                                value: setup.value,
                                index: setup.index,
                                length: setup.length,
                            },
                            TIMEOUT,
                        )
                        .await
                        .map_err(Error::Transfer)?;

                    let copied = data.len().min(out.len());
                    out[..copied].copy_from_slice(&data[..copied]);

                    Ok(TransferResult {
                        bytes_transferred: copied,
                    })
                }
                TransferBuffer::Out(data) => {
                    interface
                        .control_out(
                            ControlOut {
                                control_type,
                                recipient,
                                request: setup.request,
                                value: setup.value,
                                index: setup.index,
                                data,
                            },
                            TIMEOUT,
                        )
                        .await
                        .map_err(Error::Transfer)?;

                    Ok(TransferResult {
                        bytes_transferred: data.len(),
                    })
                }
            };
        }

        // Before an interface is claimed there is no WinUSB handle to route
        // control transfers through on Windows, so fall back to nusb's native
        // descriptor reader for standard GET_DESCRIPTOR requests.
        if setup.control_type == ControlType::Standard && setup.request == GET_DESCRIPTOR {
            let TransferBuffer::In(out) = buffer else {
                return Err(Error::Unsupported);
            };

            let device = self.device.as_ref().ok_or(Error::NotOpen)?;
            let data = device
                .get_descriptor(
                    (setup.value >> 8) as u8,
                    setup.value as u8,
                    setup.index,
                    TIMEOUT,
                )
                .await
                .map_err(Error::GetDescriptor)?;

            let copied = data.len().min(out.len());
            out[..copied].copy_from_slice(&data[..copied]);

            return Ok(TransferResult {
                bytes_transferred: copied,
            });
        }

        Err(Error::NotOpen)
    }

    async fn transfer(&mut self, transfer: Transfer<'_>) -> Result<TransferResult, Error> {
        let Transfer {
            endpoint,
            transfer_type,
            buffer,
        } = transfer;

        let interface = self.interface.as_ref().ok_or(Error::NotOpen)?;
        let address = endpoint;

        match (transfer_type, buffer) {
            (TransferType::Interrupt, TransferBuffer::In(out)) => {
                read_endpoint::<Interrupt>(interface, address, out).await
            }
            (TransferType::Interrupt, TransferBuffer::Out(data)) => {
                write_endpoint::<Interrupt>(interface, address, data).await
            }
            (TransferType::Bulk, TransferBuffer::In(out)) => {
                read_endpoint::<Bulk>(interface, address, out).await
            }
            (TransferType::Bulk, TransferBuffer::Out(data)) => {
                write_endpoint::<Bulk>(interface, address, data).await
            }
            _ => Err(Error::Unsupported),
        }
    }

    fn is_stall(error: &Error) -> bool {
        matches!(error, Error::Transfer(TransferError::Stall))
    }

    async fn clear_halt(&mut self, transfer_type: TransferType, endpoint: u8) -> Result<(), Error> {
        let interface = self.interface.as_ref().ok_or(Error::NotOpen)?;
        let inbound = endpoint & 0x80 != 0;

        let cleared = match (transfer_type, inbound) {
            (TransferType::Interrupt, true) => {
                interface
                    .endpoint::<Interrupt, In>(endpoint)
                    .map_err(Error::Nusb)?
                    .clear_halt()
                    .await
            }
            (TransferType::Interrupt, false) => {
                interface
                    .endpoint::<Interrupt, Out>(endpoint)
                    .map_err(Error::Nusb)?
                    .clear_halt()
                    .await
            }
            (TransferType::Bulk, true) => {
                interface
                    .endpoint::<Bulk, In>(endpoint)
                    .map_err(Error::Nusb)?
                    .clear_halt()
                    .await
            }
            (TransferType::Bulk, false) => {
                interface
                    .endpoint::<Bulk, Out>(endpoint)
                    .map_err(Error::Nusb)?
                    .clear_halt()
                    .await
            }
            _ => return Err(Error::Unsupported),
        };

        cleared.map_err(Error::Nusb)
    }
}

/// Submit one transfer and wait for it asynchronously, giving up after
/// `timeout`. The pending transfer is cancelled when the endpoint is dropped.
async fn transfer_with_timeout<EpType: BulkOrInterrupt, Dir: EndpointDirection>(
    endpoint: &mut Endpoint<EpType, Dir>,
    buffer: Buffer,
    timeout: Duration,
) -> Result<Completion, Error> {
    endpoint.submit(buffer);

    let next = endpoint.next_complete();
    futures::pin_mut!(next);

    match futures::future::select(next, futures_timer::Delay::new(timeout)).await {
        Either::Left((completion, _)) => Ok(completion),
        Either::Right((_, _)) => Err(Error::Transfer(TransferError::Cancelled)),
    }
}

async fn read_endpoint<T: BulkOrInterrupt>(
    interface: &Interface,
    address: u8,
    out: &mut [u8],
) -> Result<TransferResult, Error> {
    let mut endpoint = interface.endpoint::<T, In>(address).map_err(Error::Nusb)?;

    // IN transfers must request a nonzero multiple of the packet size.
    let packet = endpoint.max_packet_size().max(1);
    let request = out.len().max(1).div_ceil(packet) * packet;

    let completion = transfer_with_timeout(&mut endpoint, Buffer::new(request), TIMEOUT).await?;
    let data = completion.into_result().map_err(Error::Transfer)?;

    let copied = data.len().min(out.len());
    out[..copied].copy_from_slice(&data[..copied]);

    Ok(TransferResult {
        bytes_transferred: copied,
    })
}

async fn write_endpoint<T: BulkOrInterrupt>(
    interface: &Interface,
    address: u8,
    data: &[u8],
) -> Result<TransferResult, Error> {
    let mut endpoint = interface.endpoint::<T, Out>(address).map_err(Error::Nusb)?;

    let completion = transfer_with_timeout(&mut endpoint, Buffer::from(data), TIMEOUT).await?;
    completion.into_result().map_err(Error::Transfer)?;

    Ok(TransferResult {
        bytes_transferred: data.len(),
    })
}
