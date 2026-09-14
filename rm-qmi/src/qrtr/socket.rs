//! The socket a QRTR transport speaks through.
//!
//! The bus is a datagram socket: a message goes to a node and a port, and the
//! answers come back from whatever address they were sent to. A transport
//! needs nothing else from the link, which is what this trait says — and it is
//! a trait rather than a descriptor because opening a socket on the bus is not
//! something every process may do. On Android an app may not, and it may not
//! use one somebody else opened either, so the socket stays in a process that
//! may have it and the app hands in an implementation that asks that process
//! to send and to wait.

use core::time::Duration;

/// Where a datagram is going, or came from: a node and a port on the bus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QrtrAddress {
    pub node: u32,
    pub port: u32,
}

/// One datagram socket to the bus.
#[trait_variant::make(QrtrSocket: Send)]
pub trait LocalQrtrSocket {
    /// Failure of the link itself, below the protocol.
    type Error: core::error::Error;

    /// The address this socket sends from, which is what a lookup starts at.
    async fn address(&mut self) -> Result<QrtrAddress, Self::Error>;

    /// Send one datagram to an address on the bus.
    async fn send(&mut self, to: QrtrAddress, datagram: &[u8]) -> Result<(), Self::Error>;

    /// Receive one datagram, giving up after `timeout`.
    ///
    /// `None` when nothing arrived in time. The datagram is written at the
    /// start of `buffer`, and how long it was comes back with where it came
    /// from.
    async fn receive(
        &mut self,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<Option<(usize, QrtrAddress)>, Self::Error>;
}
