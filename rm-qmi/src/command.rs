//! The requests and responses of the QMI messages.
//!
//! A request and its response are bound to each other, and both know how to
//! encode and decode their TLV list, so the same pair serves the client that
//! sends the request and the server that answers it.

use alloc::vec::Vec;

use crate::{Error, TlvBuf};

/// One QMI request: the service and message id it goes to, and the response it
/// is answered with.
///
/// Every generated `Request` of [`crate::types`] implements this, so
/// [`Session::call`](crate::Session::call) sends one without the caller naming
/// the service or the message id.
pub trait Request: Sized {
    /// Service the request goes to.
    const SERVICE: u16;

    /// Message id inside that service.
    const MESSAGE_ID: u16;

    /// Response this request is answered with.
    type Response: Response<Request = Self>;

    /// Encode the TLV list to send.
    fn encode(&self) -> Result<Vec<TlvBuf>, Error>;

    /// Decode a TLV list that arrived for the message.
    fn decode(tlvs: &[TlvBuf]) -> Result<Self, Error>;
}

/// The response of one QMI request.
///
/// A generated response is the data of a success. Decoding an answer the modem
/// rejected reads as the protocol error it was answered with.
pub trait Response: Sized {
    /// Request this response answers.
    type Request: Request<Response = Self>;

    /// Encode the TLV list to send back.
    fn encode(&self) -> Result<Vec<TlvBuf>, Error>;

    /// Decode a TLV list that arrived for the response.
    fn decode(tlvs: &[TlvBuf]) -> Result<Self, Error>;
}
