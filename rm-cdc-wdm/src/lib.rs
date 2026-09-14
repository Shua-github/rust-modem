#![no_std]

//! CDC WDM (WMC Device Management) driver, modelled after the Linux kernel's
//! `drivers/usb/class/cdc-wdm.c`.
//!
//! The device is discovered by walking its configuration descriptor: the
//! function is the communication class interface with subclass DMM (0x09),
//! which exposes one interrupt IN endpoint for notifications and a
//! class-specific DMM functional descriptor carrying `wMaxCommand`.
//!
//! Data is exchanged through class-specific control transfers on the interface
//! (`SEND_ENCAPSULATED_COMMAND` / `GET_ENCAPSULATED_RESPONSE`); the interrupt
//! endpoint only signals that a response is pending. Reading first waits for
//! that notification, but falls back to fetching the response directly once
//! the transport's interrupt timeout expires.

extern crate alloc;

mod descriptor;
mod device;
mod error;
mod notification;

pub use descriptor::{WdmInterface, find_wdm_interface};
pub use device::CdcWdm;
pub use error::Error;
pub use notification::{
    NOTIFY_NETWORK_CONNECTION, NOTIFY_RESPONSE_AVAILABLE, NOTIFY_SERIAL_STATE, NOTIFY_SPEED_CHANGE,
    Notification,
};
