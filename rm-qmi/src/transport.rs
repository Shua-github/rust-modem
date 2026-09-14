//! The pipe a QMI client speaks over.
//!
//! QMI is a request and response protocol over a message pipe, and this trait
//! is that pipe. Every QMI message names the service it belongs to — control,
//! UIM, DMS — and how a transport gets the message to that service is the one
//! thing that differs: a CDC WDM function carries them all over the interface
//! it claimed and reads the service back out of the frame, while QRTR gives
//! each service a port of its own and therefore needs the service named before
//! it can address anything. What the client above sees is the frame either
//! way: a transport that does not carry the header takes it off and puts it
//! back.

/// How a link tells one QMI client from another.
///
/// USB frames carry a client id, and the modem is what hands them out: a
/// control message asks for one before anything else can be sent, and the
/// answer names a client the modem knows. QRTR needs none of that — a client
/// is the port its socket is bound to, the service is the port a message is
/// addressed to, and the control messages that would ask for a client id are
/// answered by the link itself rather than by the modem. A client has to know
/// which of the two it is on, or it asks a bus that publishes no control
/// service for a client id it will never hand out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexing {
    Qmux,
    Qrtr,
}

/// One message pipe to a modem's QMI services.
#[trait_variant::make(Transport: Send)]
pub trait LocalTransport {
    /// Failure of the link itself, below the protocol.
    type Error: core::error::Error;

    /// Which set of control messages this link answers.
    fn multiplexing(&self) -> Multiplexing;

    /// Send one frame belonging to `service`.
    async fn write(&mut self, service: u16, frame: &[u8]) -> Result<(), Self::Error>;

    /// Receive one frame, returning how many bytes it carried.
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error>;
}
