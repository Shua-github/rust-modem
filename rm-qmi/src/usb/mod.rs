//! QMI over USB: a CDC WDM function, and the modems that carry one.
//!
//! Everything that needs `rm-cdc-wdm` and `rm-usb-core` lives here — the
//! device table the kernel's `qmi_wwan` driver is generated from, the service
//! that tells a device's QMI function apart from the rest of its interfaces,
//! and the client that claims one. The rest of the crate is the protocol, so
//! nothing else has to know whether USB is there.

pub mod discovery;
mod table;

pub use discovery::{FILTERS, QmiEntry, QmiMatch, QmiTable, TABLE, find_qmi_interface};

use rm_cdc_wdm::CdcWdm;
use rm_usb_core::{DeviceFilter, LocalUsbClient, LocalUsbDevice};

use crate::client::{Error, QmiClient};
use crate::transport::{LocalTransport, Multiplexing};

/// A QMI client over a USB CDC WDM function.
pub type UsbQmiClient<D> = QmiClient<CdcWdm<D>>;

/// A CDC WDM function carries every service through one QMUX header, and the
/// client ids inside it are the modem's to hand out.
impl<D: LocalUsbDevice> LocalTransport for CdcWdm<D>
where
    D::Error: core::error::Error,
{
    type Error = rm_cdc_wdm::Error<D::Error>;

    fn multiplexing(&self) -> Multiplexing {
        Multiplexing::Qmux
    }

    async fn write(&mut self, _service: u16, frame: &[u8]) -> Result<(), Self::Error> {
        CdcWdm::write(self, frame).await
    }

    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        CdcWdm::read(self, out).await
    }
}

/// The USB side of the client: what a CDC WDM transport offers on top of the
/// protocol.
impl<D> QmiClient<CdcWdm<D>>
where
    D: LocalUsbDevice,
    D::Error: core::error::Error,
{
    /// Interface number the underlying WDM function is bound to.
    pub fn interface_number(&self) -> u8 {
        self.transport().interface_number()
    }

    /// Give the USB device back, dropping the claimed interface state.
    pub fn into_device(self) -> D {
        self.transport_owned().into_device()
    }
}

impl<B> LocalUsbClient for QmiClient<CdcWdm<B>>
where
    B: LocalUsbDevice,
    B::Error: core::error::Error + 'static,
{
    type Device = B;
    type Error = Error<rm_cdc_wdm::Error<B::Error>>;

    const PROTOCOL: &str = "qmi";

    const CLIENT_FILTER: &[DeviceFilter] = &FILTERS;

    async fn open(device: B) -> Result<Self, Self::Error> {
        let wdm = CdcWdm::open_with(device, |config, info| {
            find_qmi_interface(config, info, &TABLE)
        })
        .await
        .map_err(Error::Transport)?;

        Ok(QmiClient::new(wdm))
    }
}
