use crate::Error;

pub const USB_DT_CONFIG: u8 = 0x02;
pub const USB_DT_INTERFACE: u8 = 0x04;
pub const USB_DT_ENDPOINT: u8 = 0x05;
pub const USB_DT_CS_INTERFACE: u8 = 0x24;

pub const USB_CLASS_COMM: u8 = 0x02;
pub const USB_CDC_SUBCLASS_DMM: u8 = 0x09;
pub const USB_CDC_DMM_TYPE: u8 = 0x14;

pub const USB_ENDPOINT_XFER_INT: u8 = 0x03;
pub const USB_ENDPOINT_DIR_IN: u8 = 0x80;

/// `wMaxCommand` default, matching the kernel's `WDM_DEFAULT_BUFSIZE`.
pub const DEFAULT_MAX_COMMAND_SIZE: u16 = 256;

/// The communication interface and notification endpoint used by CDC WDM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WdmInterface {
    pub configuration_value: u8,
    pub interface_number: u8,
    pub notify_endpoint: u8,
    pub notify_max_packet_size: u16,
    pub notify_interval: u8,
    pub max_command_size: u16,
    pub needs_dtr: bool,
    /// Paired CDC data interface and the alternate setting that activates its
    /// data path, for functions that have one (MBIM, NCM).
    pub data_interface: Option<(u8, u8)>,
}

/// Locate the standard CDC DMM WDM function.
pub fn find_wdm_interface<E>(config: &[u8]) -> Result<WdmInterface, Error<E>> {
    if config.len() < 9 || config[1] != USB_DT_CONFIG {
        return Err(Error::InvalidDescriptor);
    }

    let configuration_value = config[5];
    let total_length = usize::from(u16::from_le_bytes([config[2], config[3]])).min(config.len());
    let mut found: Option<WdmInterface> = None;
    let mut offset = usize::from(config[0]);

    while offset + 2 <= total_length {
        let length = usize::from(config[offset]);
        let descriptor_type = config[offset + 1];
        if length < 2 || offset + length > total_length {
            return Err(Error::InvalidDescriptor);
        }

        let descriptor = &config[offset..offset + length];
        match descriptor_type {
            USB_DT_INTERFACE if length >= 9 => {
                if found.is_some() {
                    // Left the matched interface; its descriptors are complete.
                    break;
                }

                // `wdm_ids` matches on interface class and subclass only.
                let is_wdm =
                    descriptor[5] == USB_CLASS_COMM && descriptor[6] == USB_CDC_SUBCLASS_DMM;

                // `wdm_probe` rejects a DMM interface unless it has exactly one
                // endpoint, which must be the notification endpoint.
                if is_wdm && descriptor[4] == 1 {
                    found = Some(WdmInterface {
                        configuration_value,
                        interface_number: descriptor[2],
                        notify_endpoint: 0,
                        notify_max_packet_size: 0,
                        notify_interval: 0,
                        max_command_size: DEFAULT_MAX_COMMAND_SIZE,
                        needs_dtr: false,
                        data_interface: None,
                    });
                }
            }
            USB_DT_ENDPOINT if length >= 7 => {
                if let Some(current) = found.as_mut() {
                    let address = descriptor[2];
                    let attributes = descriptor[3] & 0x03;
                    if attributes == USB_ENDPOINT_XFER_INT
                        && address & USB_ENDPOINT_DIR_IN != 0
                        && current.notify_endpoint == 0
                    {
                        current.notify_endpoint = address;
                        current.notify_max_packet_size =
                            u16::from_le_bytes([descriptor[4], descriptor[5]]) & 0x07ff;
                        current.notify_interval = descriptor[6];
                    }
                }
            }
            USB_DT_CS_INTERFACE if length >= 7 => {
                if let Some(current) = found.as_mut()
                    && descriptor[2] == USB_CDC_DMM_TYPE
                {
                    current.max_command_size = u16::from_le_bytes([descriptor[5], descriptor[6]]);
                    if current.max_command_size == 0 {
                        current.max_command_size = DEFAULT_MAX_COMMAND_SIZE;
                    }
                }
            }
            _ => {}
        }

        offset += length;
    }

    match found {
        Some(interface)
            if interface.notify_endpoint != 0 && interface.notify_max_packet_size != 0 =>
        {
            Ok(interface)
        }
        Some(_) | None => Err(Error::NoWdmInterface),
    }
}
