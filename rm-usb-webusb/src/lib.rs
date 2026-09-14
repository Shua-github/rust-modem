//! WebUSB backend for the [`rm_usb_core`] traits.
//!
//! Only the browser exposes WebUSB, so this crate is wasm-only. The bindings
//! in `web-sys` are still marked unstable and require `--cfg=web_sys_unstable_apis`
//! (see `.cargo/config.toml`).

#![cfg(target_arch = "wasm32")]

use js_sys::Uint8Array;
use rm_usb_core::{
    ControlType, DeviceFilter, DeviceInfo, InterfaceInfo, LocalUsbBus, LocalUsbDevice,
    LocalUsbTransfer, Recipient, SetupPacket, Transfer, TransferBuffer, TransferResult,
    TransferType,
};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Usb, UsbAlternateInterface, UsbConfiguration, UsbControlTransferParameters, UsbDevice,
    UsbDeviceFilter, UsbDeviceRequestOptions, UsbDirection, UsbInTransferResult,
    UsbInterface as WebUsbInterface, UsbOutTransferResult, UsbRecipient, UsbRequestType,
    UsbTransferStatus,
};

/// Bound every USB operation so a modem that never answers, or a browser USB
/// backend that stalls while claiming a driver-owned interface, shows an error
/// instead of leaving the shell waiting forever. The interrupt wait is capped
/// at five seconds; `rm-cdc-wdm` then falls back to a direct
/// `GET_ENCAPSULATED_RESPONSE`, which is the path that matters on Android
/// WebUSB where RESPONSE_AVAILABLE notifications may never be delivered.
const USB_OPERATION_TIMEOUT_MS: i32 = 5_000;

/// Errors produced by the WebUSB backend.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    message: String,
    /// Set when the failure was a USB STALL.
    stalled: bool,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stalled: false,
        }
    }

    fn stall(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stalled: true,
        }
    }

    fn js(context: &str, value: JsValue) -> Self {
        let message = value
            .dyn_ref::<js_sys::Error>()
            .map(|error| String::from(error.message()))
            .or_else(|| value.as_string())
            .unwrap_or_else(|| format!("{value:?}"));

        Self::new(format!("{context}: {message}"))
    }
}

async fn settle_with_timeout<T>(
    promise: js_sys::Promise<T>,
    operation: &'static str,
) -> Result<JsValue, Error> {
    let timeout = timeout_promise(operation);
    let promises = js_sys::Array::new();
    promises.push(promise.as_ref());
    promises.push(&timeout);
    let raced = js_sys::Promise::race(&promises);
    JsFuture::from(raced)
        .await
        .map_err(|value| Error::js(operation, value))
}

fn timeout_promise(operation: &'static str) -> js_sys::Promise {
    let window = web_sys::window().expect("WebUSB requires a window");
    js_sys::Promise::new(&mut |_, reject| {
        let error = js_sys::Error::new(&format!(
            "timed out after {USB_OPERATION_TIMEOUT_MS} ms waiting for {operation}"
        ));
        let reject_error = reject.clone();
        let timeout_error = error.clone();
        let callback = Closure::once_into_js(move || {
            let _ = reject_error.call1(&JsValue::UNDEFINED, &timeout_error);
        });
        let function: &js_sys::Function = callback.as_ref().unchecked_ref();
        let scheduled = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                function,
                USB_OPERATION_TIMEOUT_MS,
            )
            .is_ok();
        if !scheduled {
            // No timer, no timeout: reject now rather than hanging forever.
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        }
    })
}

/// USB bus backed by `navigator.usb`.
pub struct WebUsbBus {}

impl WebUsbBus {
    pub fn new() -> Self {
        Self {}
    }

    fn usb() -> Result<Usb, Error> {
        let window = web_sys::window().ok_or_else(|| Error::new("no window"))?;
        let usb = window.navigator().usb();

        let raw: &JsValue = usb.as_ref();

        if raw.is_undefined() {
            return Err(Error::new(
                "WebUSB is unavailable; use Chrome or Edge over http://127.0.0.1 or https",
            ));
        }

        Ok(usb)
    }
}

impl Default for WebUsbBus {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalUsbBus for WebUsbBus {
    type Error = Error;
    type Device = WebUsbDevice;

    async fn devices(&mut self) -> Result<Vec<WebUsbDevice>, Error> {
        let usb = Self::usb()?;
        let value = JsFuture::from(usb.get_devices())
            .await
            .map_err(|value| Error::js("getDevices", value))?;
        Ok(value.iter().map(WebUsbDevice::new).collect())
    }

    /// Show the browser's device picker. Must be called from a user gesture.
    async fn request_device(&mut self, filters: &[DeviceFilter]) -> Result<WebUsbDevice, Error> {
        let usb = Self::usb()?;
        let entries = filters.iter().map(to_device_filter).collect::<Vec<_>>();
        let options = UsbDeviceRequestOptions::new(&entries);
        let device = usb
            .request_device(&options)
            .await
            .map_err(|value| Error::js("requestDevice", value))?;

        Ok(WebUsbDevice::new(device))
    }
}

/// Translate one declarative filter into the browser's `USBDeviceFilter`.
fn to_device_filter(filter: &DeviceFilter) -> UsbDeviceFilter {
    let entry = UsbDeviceFilter::new();
    if let Some(vendor) = filter.vendor_id {
        entry.set_vendor_id(vendor);
    }
    if let Some(product) = filter.product_id {
        entry.set_product_id(product);
    }
    if let Some(class) = &filter.class {
        if let Some(code) = class.class {
            entry.set_class_code(code);
        }
        if let Some(code) = class.subclass {
            entry.set_subclass_code(code);
        }
        if let Some(code) = class.protocol {
            entry.set_protocol_code(code);
        }
    }
    entry
}

/// A WebUSB device handle. Cheap to clone: the JS object is shared.
#[derive(Clone)]
pub struct WebUsbDevice {
    device: UsbDevice,
    info: DeviceInfo,
}

impl WebUsbDevice {
    pub fn new(device: UsbDevice) -> Self {
        let info = DeviceInfo {
            product_id: device.product_id(),
            vendor_id: device.vendor_id(),
            usb_version: (u16::from(device.usb_version_major()) << 8)
                | (u16::from(device.usb_version_minor()) << 4)
                | u16::from(device.usb_version_subminor()),
            class: device.device_class(),
            subclass: device.device_subclass(),
            protocol: device.device_protocol(),
            interfaces: interface_summaries(&device),
        };

        Self { device, info }
    }

    /// The underlying JS `USBDevice`.
    pub fn as_usb_device(&self) -> &UsbDevice {
        &self.device
    }
}

fn interface_summaries(device: &UsbDevice) -> Vec<InterfaceInfo> {
    let mut summaries = Vec::new();

    for configuration in device.configurations().iter() {
        let Some(configuration) = configuration.dyn_ref::<UsbConfiguration>() else {
            continue;
        };

        for interface in configuration.interfaces().iter() {
            let Some(interface) = interface.dyn_ref::<WebUsbInterface>() else {
                continue;
            };

            for alternate in interface.alternates().iter() {
                let Some(alternate) = alternate.dyn_ref::<UsbAlternateInterface>() else {
                    continue;
                };
                summaries.push(InterfaceInfo {
                    number: interface.interface_number(),
                    class: alternate.interface_class(),
                    subclass: alternate.interface_subclass(),
                    protocol: alternate.interface_protocol(),
                });
            }
        }
    }

    summaries
}

impl LocalUsbDevice for WebUsbDevice {
    type Error = Error;
    type Transfer = WebUsbTransfer;

    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    async fn open(&mut self) -> Result<(), Error> {
        settle_with_timeout(self.device.open(), "open").await?;

        Ok(())
    }

    async fn close(&mut self) -> Result<(), Error> {
        settle_with_timeout(self.device.close(), "close").await?;

        Ok(())
    }

    async fn select_configuration(&mut self, configuration: u8) -> Result<(), Error> {
        // Opening the device already selects the configuration, and selecting
        // the active one again is a no-op the browser may reject.
        if self
            .device
            .configuration()
            .map(|current| current.configuration_value())
            == Some(configuration)
        {
            return Ok(());
        }

        settle_with_timeout(
            self.device.select_configuration(configuration),
            "selectConfiguration",
        )
        .await?;

        Ok(())
    }

    async fn claim_interface(&mut self, interface: u8) -> Result<(), Error> {
        settle_with_timeout(self.device.claim_interface(interface), "claimInterface").await?;

        Ok(())
    }

    async fn release_interface(&mut self, interface: u8) -> Result<(), Error> {
        settle_with_timeout(self.device.release_interface(interface), "releaseInterface").await?;

        Ok(())
    }

    async fn set_alternate_setting(
        &mut self,
        interface: u8,
        alternate_setting: u8,
    ) -> Result<(), Error> {
        settle_with_timeout(
            UsbDevice::select_alternate_interface(&self.device, interface, alternate_setting),
            "selectAlternateInterface",
        )
        .await?;

        Ok(())
    }

    async fn reset(&mut self) -> Result<(), Error> {
        settle_with_timeout(self.device.reset(), "reset").await?;

        Ok(())
    }

    fn transfer(&mut self) -> WebUsbTransfer {
        WebUsbTransfer {
            device: self.device.clone(),
        }
    }
}

/// A single transfer issued against an open [`WebUsbDevice`].
pub struct WebUsbTransfer {
    device: UsbDevice,
}

impl LocalUsbTransfer for WebUsbTransfer {
    type Error = Error;

    async fn control_transfer(
        &mut self,
        setup: SetupPacket,
        buffer: TransferBuffer<'_>,
    ) -> Result<TransferResult, Error> {
        let parameters = UsbControlTransferParameters::new(
            setup.index,
            to_recipient(setup.recipient),
            setup.request,
            to_request_type(setup.control_type),
            setup.value,
        );

        // The setup packet's direction is implicit in WebUSB: the direction is
        // chosen by which of the two transfer calls is used, so it comes from
        // the buffer the caller passed.
        match buffer {
            TransferBuffer::In(out) => {
                let promise = self
                    .device
                    .control_transfer_in(&parameters, out.len() as u16);
                let value = settle_with_timeout(promise, "controlTransferIn").await?;
                let result = value
                    .dyn_into::<UsbInTransferResult>()
                    .map_err(|_| Error::new("controlTransferIn returned an unexpected result"))?;

                copy_into(&result, out)
            }
            TransferBuffer::Out(data) => {
                let promise = self
                    .device
                    .control_transfer_out_with_u8_slice(&parameters, data)
                    .map_err(|value| Error::js("controlTransferOut", value))?;
                let value = settle_with_timeout(promise, "controlTransferOut").await?;
                let result = value
                    .dyn_into::<UsbOutTransferResult>()
                    .map_err(|_| Error::new("controlTransferOut returned an unexpected result"))?;

                check_status(result.status())?;

                Ok(TransferResult {
                    bytes_transferred: result.bytes_written() as usize,
                })
            }
        }
    }

    async fn transfer(&mut self, transfer: Transfer<'_>) -> Result<TransferResult, Error> {
        let Transfer {
            endpoint,
            transfer_type,
            buffer,
        } = transfer;

        // The endpoint field is an address, with the direction in its top bit,
        // while WebUSB addresses endpoints by number and takes the direction
        // from the call.
        let number = endpoint & 0x0f;

        match (transfer_type, buffer) {
            (TransferType::Interrupt | TransferType::Bulk, TransferBuffer::In(out)) => {
                let promise = self.device.transfer_in(number, out.len() as u32);
                let value = settle_with_timeout(promise, "transferIn").await?;
                let result = value
                    .dyn_into::<UsbInTransferResult>()
                    .map_err(|_| Error::new("transferIn returned an unexpected result"))?;

                copy_into(&result, out)
            }
            (TransferType::Interrupt | TransferType::Bulk, TransferBuffer::Out(data)) => {
                let promise = self
                    .device
                    .transfer_out_with_u8_slice(number, data)
                    .map_err(|value| Error::js("transferOut", value))?;
                let value = settle_with_timeout(promise, "transferOut").await?;
                let result = value
                    .dyn_into::<UsbOutTransferResult>()
                    .map_err(|_| Error::new("transferOut returned an unexpected result"))?;

                check_status(result.status())?;

                Ok(TransferResult {
                    bytes_transferred: result.bytes_written() as usize,
                })
            }
            _ => Err(Error::new("unsupported transfer type")),
        }
    }

    fn is_stall(error: &Error) -> bool {
        error.stalled
    }

    async fn clear_halt(
        &mut self,
        _transfer_type: TransferType,
        endpoint: u8,
    ) -> Result<(), Error> {
        let direction = if endpoint & 0x80 != 0 {
            UsbDirection::In
        } else {
            UsbDirection::Out
        };

        settle_with_timeout(
            self.device.clear_halt(direction, endpoint & 0x0f),
            "clearHalt",
        )
        .await?;

        Ok(())
    }
}

/// Translate the setup packet's request type into the browser's enum.
fn to_request_type(control_type: ControlType) -> UsbRequestType {
    match control_type {
        ControlType::Standard => UsbRequestType::Standard,
        ControlType::Class => UsbRequestType::Class,
        ControlType::Vendor => UsbRequestType::Vendor,
    }
}

/// Translate the setup packet's recipient into the browser's enum.
fn to_recipient(recipient: Recipient) -> UsbRecipient {
    match recipient {
        Recipient::Device => UsbRecipient::Device,
        Recipient::Interface => UsbRecipient::Interface,
        Recipient::Endpoint => UsbRecipient::Endpoint,
        Recipient::Other => UsbRecipient::Other,
    }
}

fn check_status(status: UsbTransferStatus) -> Result<(), Error> {
    if status == UsbTransferStatus::Ok {
        Ok(())
    } else if status == UsbTransferStatus::Stall {
        Err(Error::stall(format!("usb transfer failed: {status:?}")))
    } else {
        Err(Error::new(format!("usb transfer failed: {status:?}")))
    }
}

fn copy_into(result: &UsbInTransferResult, out: &mut [u8]) -> Result<TransferResult, Error> {
    check_status(result.status())?;

    let Some(view) = result.data() else {
        return Ok(TransferResult {
            bytes_transferred: 0,
        });
    };

    let bytes = Uint8Array::new_with_byte_offset_and_length(
        &view.buffer(),
        view.byte_offset() as u32,
        view.byte_length() as u32,
    );
    let copied = bytes.length() as usize;
    let copied = copied.min(out.len());
    bytes.subarray(0, copied as u32).copy_to(&mut out[..copied]);

    Ok(TransferResult {
        bytes_transferred: copied,
    })
}
