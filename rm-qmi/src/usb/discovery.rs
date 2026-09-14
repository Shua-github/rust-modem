//! QMI function discovery, kept with the QMI protocol rather than generic
//! CDC WDM transport.

use rm_cdc_wdm::{WdmInterface, find_wdm_interface};
use rm_usb_core::{ClassFilter, DeviceFilter, DeviceInfo, InterfaceInfo};

use super::table::{COUNT, ENTRIES};

const USB_DT_CONFIG: u8 = 0x02;
const USB_DT_INTERFACE: u8 = 0x04;
const USB_DT_ENDPOINT: u8 = 0x05;
const USB_CLASS_VENDOR_SPEC: u8 = 0xff;
const USB_ENDPOINT_XFER_INT: u8 = 0x03;
const USB_ENDPOINT_DIR_IN: u8 = 0x80;
const DEFAULT_MAX_COMMAND_SIZE: u16 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QmiEntry {
    pub vendor: u16,
    pub product: Option<u16>,
    pub kind: QmiMatch,
    pub set_dtr: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QmiMatch {
    Interface(u8),
    Class {
        class: u8,
        subclass: u8,
        protocol: u8,
    },
}

impl QmiEntry {
    /// Fixed interface match, from `QMI_FIXED_INTF()` and friends.
    pub const fn interface(vendor: u16, product: Option<u16>, number: u8, set_dtr: bool) -> Self {
        Self {
            vendor,
            product,
            kind: QmiMatch::Interface(number),
            set_dtr,
        }
    }

    /// Class match, from `USB_DEVICE_AND_INTERFACE_INFO()` and friends.
    pub const fn class(
        vendor: u16,
        product: Option<u16>,
        class: u8,
        subclass: u8,
        protocol: u8,
        set_dtr: bool,
    ) -> Self {
        Self {
            vendor,
            product,
            kind: QmiMatch::Class {
                class,
                subclass,
                protocol,
            },
            set_dtr,
        }
    }

    /// USB filter that picks this entry's devices out of a device list.
    pub const fn filter(&self) -> DeviceFilter {
        let (class, subclass, protocol) = match self.kind {
            QmiMatch::Interface(_) => (None, None, None),
            QmiMatch::Class {
                class,
                subclass,
                protocol,
            } => (Some(class), Some(subclass), Some(protocol)),
        };

        DeviceFilter {
            vendor_id: Some(self.vendor),
            product_id: self.product,
            class: Some(ClassFilter {
                class,
                subclass,
                protocol,
            }),
            interface: None,
        }
    }
}

impl QmiEntry {
    fn matches(&self, vendor: u16, product: u16, interface: &InterfaceInfo) -> bool {
        if self.vendor != vendor || self.product.is_some_and(|id| id != product) {
            return false;
        }

        match self.kind {
            QmiMatch::Interface(number) => {
                number == interface.number && interface.class == USB_CLASS_VENDOR_SPEC
            }
            QmiMatch::Class {
                class,
                subclass,
                protocol,
            } => {
                class == interface.class
                    && subclass == interface.subclass
                    && protocol == interface.protocol
            }
        }
    }
}

/// A set of [`QmiEntry`] records to match devices against.
#[derive(Debug, Clone, Copy, Default)]
pub struct QmiTable(&'static [QmiEntry]);

impl QmiTable {
    pub const fn new(entries: &'static [QmiEntry]) -> Self {
        Self(entries)
    }

    pub const fn entries(&self) -> &'static [QmiEntry] {
        self.0
    }

    fn find(&self, vendor: u16, product: u16, interface: &InterfaceInfo) -> Option<&QmiEntry> {
        self.0
            .iter()
            .find(|entry| entry.matches(vendor, product, interface))
    }
}

/// The vendor table compiled into this crate.
pub const TABLE: QmiTable = QmiTable::new(&ENTRIES);

/// Generic CDC DMM match (class 0x02, subclass 0x09), which carries the QMI
/// function on devices that do not need a vendor quirk.
const CDC_DMM: DeviceFilter = DeviceFilter::class(0x02, 0x09, 0x00);

/// USB filters that select a QMI function: the generic CDC DMM match followed
/// by one filter per vendor table entry, all computed at compile time.
pub const FILTERS: [DeviceFilter; COUNT + 1] = build_filters();

/// Turn every vendor table entry into a USB filter, behind the CDC DMM match.
const fn build_filters() -> [DeviceFilter; COUNT + 1] {
    let mut filters = [CDC_DMM; COUNT + 1];
    let mut index = 0;
    while index < COUNT {
        filters[index + 1] = ENTRIES[index].filter();
        index += 1;
    }
    filters
}

pub fn find_qmi_interface(
    config: &[u8],
    device: &DeviceInfo,
    table: &QmiTable,
) -> Option<WdmInterface> {
    find_vendor_interface(config, device, table).or_else(|| find_wdm_interface::<()>(config).ok())
}

fn find_vendor_interface(
    config: &[u8],
    device: &DeviceInfo,
    table: &QmiTable,
) -> Option<WdmInterface> {
    if config.len() < 9 || config[1] != USB_DT_CONFIG {
        return None;
    }

    let vendor = device.vendor_id;
    let product = device.product_id;
    let configuration_value = config[5];
    let interface_count = config[4];
    let total_length = usize::from(u16::from_le_bytes([config[2], config[3]])).min(config.len());
    let ec20 = vendor == 0x05c6 && product == 0x9215 && interface_count == 5;
    let mut offset = usize::from(config[0]);

    while offset + 2 <= total_length {
        let length = usize::from(config[offset]);
        let descriptor_type = config[offset + 1];
        if length < 2 || offset + length > total_length {
            return None;
        }

        if descriptor_type == USB_DT_INTERFACE && length >= 9 {
            let descriptor = &config[offset..offset + length];
            let interface = InterfaceInfo {
                number: descriptor[2],
                class: descriptor[5],
                subclass: descriptor[6],
                protocol: descriptor[7],
            };
            let endpoints = descriptor[4];
            let skip = (ec20 && interface.number == 0) || endpoints == 2;

            if !skip
                && let Some(quirk) = table.find(vendor, product, &interface)
                && let Some(notification) =
                    interrupt_in_endpoint(config, offset + length, endpoints, total_length)
            {
                return Some(WdmInterface {
                    configuration_value,
                    interface_number: interface.number,
                    notify_endpoint: notification.address,
                    notify_max_packet_size: notification.max_packet_size,
                    notify_interval: notification.interval,
                    max_command_size: DEFAULT_MAX_COMMAND_SIZE,
                    needs_dtr: quirk.set_dtr || device.usb_version >= 0x0201,
                    data_interface: None,
                });
            }
        }

        offset += length;
    }

    None
}

struct NotificationEndpoint {
    address: u8,
    max_packet_size: u16,
    interval: u8,
}

fn interrupt_in_endpoint(
    config: &[u8],
    mut offset: usize,
    endpoints: u8,
    total_length: usize,
) -> Option<NotificationEndpoint> {
    let mut remaining = endpoints;
    while remaining > 0 && offset + 2 <= total_length {
        let length = usize::from(config[offset]);
        let descriptor_type = config[offset + 1];
        if length < 2 || offset + length > total_length || descriptor_type == USB_DT_INTERFACE {
            return None;
        }
        if descriptor_type == USB_DT_ENDPOINT && length >= 7 {
            remaining -= 1;
            let address = config[offset + 2];
            let attributes = config[offset + 3] & 0x03;
            if attributes == USB_ENDPOINT_XFER_INT && address & USB_ENDPOINT_DIR_IN != 0 {
                return Some(NotificationEndpoint {
                    address,
                    max_packet_size: u16::from_le_bytes([config[offset + 4], config[offset + 5]])
                        & 0x07ff,
                    interval: config[offset + 6],
                });
            }
        }
        offset += length;
    }
    None
}
