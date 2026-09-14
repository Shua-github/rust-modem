use alloc::vec;
use alloc::vec::Vec;

use rm_usb_core::{
    ControlType, DeviceInfo, DirectionType, LocalUsbDevice, LocalUsbTransfer, Recipient,
    SetupPacket, Transfer, TransferBuffer, TransferType,
};

use crate::Error;
use crate::descriptor::{USB_DT_CONFIG, WdmInterface, find_wdm_interface};
use crate::notification::{NOTIFY_RESPONSE_AVAILABLE, Notification};

const GET_DESCRIPTOR: u8 = 0x06;
const SEND_ENCAPSULATED_COMMAND: u8 = 0x00;
const GET_ENCAPSULATED_RESPONSE: u8 = 0x01;
const SET_CONTROL_LINE_STATE: u8 = 0x22;
const SEND_BREAK: u8 = 0x23;

/// A generic CDC WDM function. Protocol-specific discovery is supplied by the
/// caller through [`Self::open_with`].
pub struct CdcWdm<D: LocalUsbDevice> {
    device: D,
    interface: u8,
    notify_endpoint: u8,
    max_command_size: u16,
    notify_buffer: Vec<u8>,
    response_buffer: Vec<u8>,
    pending: Vec<u8>,
    pending_offset: usize,
}

impl<D: LocalUsbDevice> CdcWdm<D> {
    /// Open a standard CDC DMM WDM function.
    pub async fn new(device: D) -> Result<Self, Error<D::Error>> {
        Self::open_with(device, |config, _| {
            find_wdm_interface::<D::Error>(config).ok()
        })
        .await
    }

    /// Open a WDM function selected by a protocol-specific discovery routine.
    ///
    /// The routine only examines the configuration descriptor and device info;
    /// all USB opening, configuration selection and interface claiming remain
    /// in this generic transport crate.
    pub async fn open_with<F>(mut device: D, finder: F) -> Result<Self, Error<D::Error>>
    where
        F: FnOnce(&[u8], &DeviceInfo) -> Option<WdmInterface>,
    {
        device.open().await.map_err(Error::Usb)?;

        let mut header = [0u8; 9];
        let read = Self::get_descriptor(&mut device, USB_DT_CONFIG, 0, &mut header).await?;
        if read < 9 {
            return Err(Error::InvalidDescriptor);
        }

        let total_length = usize::from(u16::from_le_bytes([header[2], header[3]]));
        let mut config = vec![0u8; total_length];
        let read = Self::get_descriptor(&mut device, USB_DT_CONFIG, 0, &mut config).await?;
        let interface = finder(&config[..read], &device.info()).ok_or(Error::NoWdmInterface)?;

        // Claim first. The device is normally already configured by the time it
        // is opened, and repeating SET_CONFIGURATION is not harmless everywhere:
        // an Android QMI function re-initialises on it and leaves its
        // notification endpoint halted, which then costs a clear-halt that a
        // usbfs/usbip transport may not survive. Select the configuration only
        // when the interface cannot be claimed as it stands.
        if device
            .claim_interface(interface.interface_number)
            .await
            .is_err()
        {
            device
                .select_configuration(interface.configuration_value)
                .await
                .map_err(Error::Usb)?;
            device
                .claim_interface(interface.interface_number)
                .await
                .map_err(Error::Usb)?;
        }

        // CDC MBIM keeps its bulk data path on a paired interface, and the
        // kernel's cdc_mbim selects that interface's data altsetting when it
        // binds. A modem whose data path was never activated accepts
        // SEND_ENCAPSULATED_COMMAND but never answers it, so do the same here.
        if let Some((data, alternate)) = interface.data_interface {
            device.claim_interface(data).await.map_err(Error::Usb)?;
            device
                .set_alternate_setting(data, alternate)
                .await
                .map_err(Error::Usb)?;
        }

        let needs_dtr = interface.needs_dtr;
        let mut wdm = Self {
            device,
            interface: interface.interface_number,
            notify_endpoint: interface.notify_endpoint,
            max_command_size: interface.max_command_size,
            notify_buffer: vec![0u8; usize::from(interface.notify_max_packet_size)],
            response_buffer: vec![0u8; usize::from(interface.max_command_size)],
            pending: Vec::new(),
            pending_offset: 0,
        };

        if needs_dtr {
            wdm.set_control_line_state(true, false).await?;
        }

        Ok(wdm)
    }

    pub const fn max_command_size(&self) -> u16 {
        self.max_command_size
    }

    pub const fn interface_number(&self) -> u8 {
        self.interface
    }

    pub fn into_device(self) -> D {
        self.device
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), Error<D::Error>> {
        // `wdm_write` truncates oversized writes to wMaxCommand rather than
        // failing them, so align with that.
        let data = &data[..data.len().min(usize::from(self.max_command_size))];

        let setup = SetupPacket {
            recipient: Recipient::Interface,
            control_type: ControlType::Class,
            direction: DirectionType::Out,
            request: SEND_ENCAPSULATED_COMMAND,
            value: 0,
            index: u16::from(self.interface),
            length: data.len() as u16,
        };
        let mut transfer = self.device.transfer();
        transfer
            .control_transfer(setup, TransferBuffer::Out(data))
            .await
            .map_err(Error::Usb)?;
        Ok(())
    }

    pub async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error<D::Error>> {
        loop {
            if self.pending_offset < self.pending.len() {
                let available = &self.pending[self.pending_offset..];
                let copied = available.len().min(out.len());
                out[..copied].copy_from_slice(&available[..copied]);
                self.pending_offset += copied;
                return Ok(copied);
            }

            // The buffer has been read empty: wait for the device to announce
            // its next response, then take exactly the one it is offering.
            //
            // The announcement is only a hint. A device may never send one, or
            // may leave its notification endpoint halted, so a failed wait is
            // followed by one direct `GET_ENCAPSULATED_RESPONSE`; the wait
            // failure is only reported when that read comes back empty.
            //
            // Reading `GET_ENCAPSULATED_RESPONSE` again until it comes back
            // empty, the way `wdm_in_callback` drains a burst, is not portable:
            // a host that has no pending response reports that extra read as a
            // transfer error (WebUSB does) instead of an empty transfer.
            self.pending.clear();
            self.pending_offset = 0;
            match self.wait_for_response_available().await {
                Ok(()) => self.fetch_response().await?,
                Err(error) => {
                    self.fetch_response().await?;
                    if self.pending.is_empty() {
                        return Err(error);
                    }
                }
            }
        }
    }

    async fn fetch_response(&mut self) -> Result<(), Error<D::Error>> {
        let setup = SetupPacket {
            recipient: Recipient::Interface,
            control_type: ControlType::Class,
            direction: DirectionType::In,
            request: GET_ENCAPSULATED_RESPONSE,
            value: 0,
            index: u16::from(self.interface),
            length: self.max_command_size,
        };
        let mut transfer = self.device.transfer();
        let result = transfer
            .control_transfer(setup, TransferBuffer::In(&mut self.response_buffer))
            .await;

        let result = match result {
            Ok(result) => result,
            // `wdm_in_callback` treats a stalled response read as an empty
            // read, so a device that stalls instead of answering leaves the
            // buffer empty for the next notification rather than failing.
            Err(error) if <D::Transfer as LocalUsbTransfer>::is_stall(&error) => return Ok(()),
            Err(error) => return Err(Error::Usb(error)),
        };

        self.pending
            .extend_from_slice(&self.response_buffer[..result.bytes_transferred]);
        Ok(())
    }

    pub async fn set_control_line_state(
        &mut self,
        dtr: bool,
        rts: bool,
    ) -> Result<(), Error<D::Error>> {
        let value = u16::from(dtr) | (u16::from(rts) << 1);
        let setup = SetupPacket {
            recipient: Recipient::Interface,
            control_type: ControlType::Class,
            direction: DirectionType::Out,
            request: SET_CONTROL_LINE_STATE,
            value,
            index: u16::from(self.interface),
            length: 0,
        };
        let mut transfer = self.device.transfer();
        transfer
            .control_transfer(setup, TransferBuffer::Out(&[]))
            .await
            .map_err(Error::Usb)?;
        Ok(())
    }

    pub async fn send_break(&mut self, duration_ms: u16) -> Result<(), Error<D::Error>> {
        let setup = SetupPacket {
            recipient: Recipient::Interface,
            control_type: ControlType::Class,
            direction: DirectionType::Out,
            request: SEND_BREAK,
            value: duration_ms,
            index: u16::from(self.interface),
            length: 0,
        };

        let mut transfer = self.device.transfer();
        transfer
            .control_transfer(setup, TransferBuffer::Out(&[]))
            .await
            .map_err(Error::Usb)?;
        Ok(())
    }

    async fn wait_for_response_available(&mut self) -> Result<(), Error<D::Error>> {
        loop {
            let transfer = Transfer {
                endpoint: self.notify_endpoint,
                transfer_type: TransferType::Interrupt,
                buffer: TransferBuffer::In(&mut self.notify_buffer),
            };
            let mut usb = self.device.transfer();
            let result = usb.transfer(transfer).await;

            let result = match result {
                Ok(result) => result,
                // `wdm_int_callback` clears the halt on a stalled notification
                // endpoint and resubmits, otherwise the endpoint stays dead.
                Err(error) if <D::Transfer as LocalUsbTransfer>::is_stall(&error) => {
                    usb.clear_halt(TransferType::Interrupt, self.notify_endpoint)
                        .await
                        .map_err(Error::Usb)?;
                    continue;
                }
                Err(error) => return Err(Error::Usb(error)),
            };

            let notification =
                Notification::parse(&self.notify_buffer[..result.bytes_transferred])?;
            if notification.kind == NOTIFY_RESPONSE_AVAILABLE {
                return Ok(());
            }
        }
    }

    async fn get_descriptor(
        device: &mut D,
        descriptor_type: u8,
        index: u8,
        buffer: &mut [u8],
    ) -> Result<usize, Error<D::Error>> {
        let setup = SetupPacket {
            recipient: Recipient::Device,
            control_type: ControlType::Standard,
            direction: DirectionType::In,
            request: GET_DESCRIPTOR,
            value: (u16::from(descriptor_type) << 8) | u16::from(index),
            index: 0,
            length: buffer.len() as u16,
        };
        let mut transfer = device.transfer();
        let result = transfer
            .control_transfer(setup, TransferBuffer::In(buffer))
            .await
            .map_err(Error::Usb)?;
        Ok(result.bytes_transferred)
    }
}
