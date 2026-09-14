//! QMI over QRTR, the router Qualcomm modems publish their services on.
//!
//! QRTR is not USB: the modem's services are ports on a bus, and reaching one
//! means asking the bus's name service where it is. What the bus carries is the
//! QMI message alone — the service is the port a message is sent to, and the
//! port it came from is what names the service in the answer — so this layer
//! takes the transport header off on the way out and puts one back on the way
//! in, and nothing above it needs to know which of the two transports it is
//! running on.
//!
//! The socket itself is behind [`LocalQrtrSocket`]: a process the bus lets in
//! uses [`BusSocket`] and talks to it directly, and a process it does not — an
//! Android app — hands in an implementation that asks one that does.

mod socket;
mod transport;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod bus;

pub use socket::{LocalQrtrSocket, QrtrAddress};
pub use transport::{Error, QrtrTransport};

#[cfg(any(target_os = "linux", target_os = "android"))]
pub use bus::BusSocket;
