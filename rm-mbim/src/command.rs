//! The requests and responses of the MBIM commands.
//!
//! A request and its response are bound to each other, and both know how to
//! encode and decode their information buffer, so the same pair serves the
//! client that sends the request and the server that answers it.

use alloc::vec::Vec;

use crate::{CommandType, Error, Uuid};

/// One MBIM request: the service and command id it goes to, whether it is a
/// query or a set, and the response it is answered with.
///
/// Every `QueryRequest` and `SetRequest` of [`crate::types`] implements this,
/// so [`Session::call`](crate::Session::call) sends one without the caller
/// naming the service, the command id or the command type.
pub trait Request: Sized {
    /// Service the request belongs to.
    const SERVICE: Uuid;

    /// Command id inside that service.
    const CID: u32;

    /// Query or set.
    const COMMAND_TYPE: CommandType;

    /// Response this request is answered with.
    type Response: Response<Request = Self>;

    /// Encode the information buffer to send.
    fn encode(&self) -> Vec<u8>;

    /// Decode an information buffer received for the command.
    fn decode(bytes: &[u8]) -> Result<Self, Error>;
}

/// The response of one MBIM request.
pub trait Response: Sized {
    /// Request this response answers.
    type Request: Request<Response = Self>;

    /// Encode the information buffer to send back.
    fn encode(&self) -> Vec<u8>;

    /// Decode an information buffer received for the response.
    fn decode(bytes: &[u8]) -> Result<Self, Error>;
}
