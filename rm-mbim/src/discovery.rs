//! CDC MBIM interface discovery, kept with the MBIM protocol.

use rm_cdc_wdm::WdmInterface;
use rm_usb_core::DeviceInfo;

const USB_DT_CONFIG: u8 = 0x02;
const USB_DT_INTERFACE: u8 = 0x04;
const USB_DT_ENDPOINT: u8 = 0x05;
const USB_DT_CS_INTERFACE: u8 = 0x24;
const USB_CLASS_COMM: u8 = 0x02;
const USB_CDC_SUBCLASS_MBIM: u8 = 0x0e;
const USB_CDC_PROTO_NONE: u8 = 0;
const USB_CDC_MBIM_TYPE: u8 = 0x1b;
const USB_CDC_UNION_TYPE: u8 = 0x06;
const USB_ENDPOINT_XFER_INT: u8 = 0x03;
const USB_ENDPOINT_DIR_IN: u8 = 0x80;
const DEFAULT_MAX_COMMAND_SIZE: u16 = 4096;
/// Alternate setting that activates the MBIM data path, matching the kernel's
/// `CDC_NCM_DATA_ALTSETTING_MBIM`.
const MBIM_DATA_ALTSETTING: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceSummary {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

pub fn is_mbim_device(interfaces: &[InterfaceSummary]) -> bool {
    interfaces.iter().any(|interface| {
        interface.class == USB_CLASS_COMM
            && interface.subclass == USB_CDC_SUBCLASS_MBIM
            && interface.protocol == USB_CDC_PROTO_NONE
    })
}

pub fn find_mbim_interface(config: &[u8], _device: &DeviceInfo) -> Option<WdmInterface> {
    if config.len() < 9 || config[1] != USB_DT_CONFIG {
        return None;
    }

    let configuration_value = config[5];
    let total_length = usize::from(u16::from_le_bytes([config[2], config[3]])).min(config.len());
    let mut found = None;
    let mut descriptor_found = false;
    let mut offset = usize::from(config[0]);

    while offset + 2 <= total_length {
        let length = usize::from(config[offset]);
        let descriptor_type = config[offset + 1];
        if length < 2 || offset + length > total_length {
            return None;
        }
        let descriptor = &config[offset..offset + length];

        match descriptor_type {
            USB_DT_INTERFACE if length >= 9 => {
                let number = descriptor[2];
                let is_mbim = descriptor[5] == USB_CLASS_COMM
                    && descriptor[6] == USB_CDC_SUBCLASS_MBIM
                    && descriptor[7] == USB_CDC_PROTO_NONE;
                if is_mbim && found.is_none() {
                    found = Some(WdmInterface {
                        configuration_value,
                        interface_number: number,
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
                if let Some(interface) = found.as_mut() {
                    let address = descriptor[2];
                    if descriptor[3] & 0x03 == USB_ENDPOINT_XFER_INT
                        && address & USB_ENDPOINT_DIR_IN != 0
                        && interface.notify_endpoint == 0
                    {
                        interface.notify_endpoint = address;
                        interface.notify_max_packet_size =
                            u16::from_le_bytes([descriptor[4], descriptor[5]]) & 0x07ff;
                        interface.notify_interval = descriptor[6];
                    }
                }
            }
            USB_DT_CS_INTERFACE if length >= 12 => {
                if let Some(interface) = found.as_mut()
                    && descriptor[2] == USB_CDC_MBIM_TYPE
                {
                    descriptor_found = true;
                    interface.max_command_size = u16::from_le_bytes([descriptor[5], descriptor[6]]);
                    if interface.max_command_size == 0 {
                        interface.max_command_size = DEFAULT_MAX_COMMAND_SIZE;
                    }
                }
            }
            // The CDC Union names the data interface paired with this function,
            // whose data altsetting must be selected before the modem answers.
            USB_DT_CS_INTERFACE if length >= 5 && descriptor[2] == USB_CDC_UNION_TYPE => {
                if let Some(interface) = found.as_mut() {
                    interface.data_interface = Some((descriptor[4], MBIM_DATA_ALTSETTING));
                }
            }
            _ => {}
        }
        offset += length;
    }

    found.filter(|interface| {
        descriptor_found && interface.notify_endpoint != 0 && interface.notify_max_packet_size != 0
    })
}
