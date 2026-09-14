//! QMI client bound to one transport, including the shared UIM interface.
//!
//! The client knows what a QMI frame looks like and nothing about what carries
//! it: the transport is `rm-transport-core`, which over USB is a CDC WDM
//! function ("[`CdcWdm`]") and over QRTR is a set of ports on the bus
//! ("[`rm_qrtr::QrtrTransport`]").

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use rm_client_core::{LocalUim, SlotState};

use crate::transport::{LocalTransport, Multiplexing};
use crate::types::{ctl, dms, uim};
use crate::{Message, Request, Response, Service, TlvBuf};

/// `QmiUimCardState::Absent`.
const CARD_STATE_ABSENT: u8 = 0;
/// `QmiUimCardApplicationState::Ready`.
const CARD_APPLICATION_STATE_READY: u8 = 7;

/// The answer type of one request: its data, or the error it was rejected
/// with.
type Answer<T> = <T as Request>::Response;

/// Errors produced by the QMI client.
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    /// The transport under the client failed.
    #[error("transport error: {0}")]
    Transport(#[source] E),
    /// A QMI message could not be encoded or decoded.
    #[error("QMI protocol error: {0}")]
    Qmi(#[source] crate::Error),
    /// The modem did not grant a client id.
    #[error("modem did not return a client id")]
    AllocationFailed,
    /// The modem did not return a value the command needs.
    #[error("modem did not return {0}")]
    Missing(&'static str),
    /// The service cannot be named on this link.
    #[error("service {0} cannot be addressed on this link")]
    UnsupportedService(u16),
}

/// A QMI client bound to one transport.
pub struct QmiClient<T: LocalTransport> {
    transport: T,
    transaction: u8,
    client_ids: Vec<(u16, u8)>,
    rx: Vec<u8>,
}

impl<T: LocalTransport> QmiClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            transaction: 0,
            client_ids: Vec::new(),
            rx: Vec::new(),
        }
    }

    /// The transport under this client, for the modules that know what it is.
    #[cfg(feature = "usb")]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }

    /// Take the transport out, for the modules that know what it is.
    #[cfg(feature = "usb")]
    pub(crate) fn transport_owned(self) -> T {
        self.transport
    }

    /// Run `f` in a QMI session.
    ///
    /// QMI has no open/close handshake, so a session is only the transaction
    /// id its requests are tagged with.
    ///
    /// ```ignore
    /// client
    ///     .with_session(async |session| {
    ///         let response = session
    ///             .call(uim::messages::get_card_status::Request::default())
    ///             .await?;
    ///         Ok(response.card_status)
    ///     })
    ///     .await
    /// ```
    pub async fn with_session<U, F>(&mut self, f: F) -> Result<U, Error<T::Error>>
    where
        F: AsyncFnOnce(&mut Session<'_, T>) -> Result<U, Error<T::Error>>,
    {
        let transaction = self.next_transaction();
        let mut session = Session {
            client: self,
            transaction,
        };

        f(&mut session).await
    }

    /// Send a request tagged with `transaction` and wait for its response.
    async fn exchange(
        &mut self,
        transaction: u16,
        service: u16,
        message_id: u16,
        tlvs: Vec<TlvBuf>,
    ) -> Result<Message, Error<T::Error>> {
        let client_id = self.client_id(service).await?;

        self.exchange_with(transaction, service, client_id, message_id, tlvs)
            .await
    }

    /// Send a request over an already resolved client id.
    ///
    /// Separate from [`Self::exchange`] because resolving a client id is
    /// itself a control message, which would otherwise recurse.
    async fn exchange_with(
        &mut self,
        transaction: u16,
        service: u16,
        client_id: u8,
        message_id: u16,
        tlvs: Vec<TlvBuf>,
    ) -> Result<Message, Error<T::Error>> {
        let request = Message::new(service, client_id, transaction, message_id).with_tlvs(tlvs);

        // One framing for every link: a transport that does not carry the
        // header itself — QRTR names the service by the port it sends to —
        // takes it off and puts it back, see [`crate::qrtr`].
        let frame = request.to_qmux_bytes();

        self.transport
            .write(service, &frame)
            .await
            .map_err(Error::Transport)?;

        loop {
            let response = self.read_message().await?;

            // Skip indications and responses to other requests.
            if response.service != service || response.transaction_id != transaction {
                continue;
            }

            return Ok(response);
        }
    }

    /// Look up (or allocate) the client id for a service. Control messages
    /// always use client id 0.
    async fn client_id(&mut self, service: u16) -> Result<u8, Error<T::Error>> {
        if service == Service::Ctl as u16 {
            return Ok(0);
        }

        if let Some(&(_, client_id)) = self.client_ids.iter().find(|(id, _)| *id == service) {
            return Ok(client_id);
        }

        let transaction = self.next_transaction();

        // The two kinds of link allocate client ids with different control
        // messages, and a link only answers the one that belongs to it.
        let cid = match self.transport.multiplexing() {
            Multiplexing::Qmux => {
                let request = ctl::messages::allocate_cid::Request {
                    service: u8::try_from(service)
                        .map_err(|_| Error::UnsupportedService(service))?,
                };
                let response = self
                    .exchange_with(
                        transaction,
                        Service::Ctl as u16,
                        0,
                        ctl::messages::allocate_cid::MESSAGE_ID,
                        request.to_tlvs().map_err(Error::Qmi)?,
                    )
                    .await?;
                let answer = <Answer<ctl::messages::allocate_cid::Request> as Response>::decode(
                    &response.tlvs,
                )
                .map_err(Error::Qmi)?;

                answer.allocation_info.cid
            }
            Multiplexing::Qrtr => {
                // A QRTR client is the port its socket is bound to, not a name
                // the modem hands out, so there is nothing to ask for: the
                // control messages that would ask are answered by the link
                // itself, and no client id ever reaches the wire. The one
                // below only fills the frame the transport is about to take
                // apart.
                0
            }
        };

        self.client_ids.push((service, cid));

        Ok(cid)
    }

    async fn read_message(&mut self) -> Result<Message, Error<T::Error>> {
        loop {
            if let Some(message) = self.take_message()? {
                return Ok(message);
            }

            let mut chunk = [0u8; 4096];
            let read = self
                .transport
                .read(&mut chunk)
                .await
                .map_err(Error::Transport)?;
            self.rx.extend_from_slice(&chunk[..read]);
        }
    }

    /// Take one complete transport frame off the receive buffer.
    fn take_message(&mut self) -> Result<Option<Message>, Error<T::Error>> {
        if self.rx.len() < 3 {
            return Ok(None);
        }

        // The length field covers everything after the marker byte.
        let length = usize::from(u16::from_le_bytes([self.rx[1], self.rx[2]]));
        let total = 1 + length;

        if self.rx.len() < total {
            return Ok(None);
        }

        let message = Message::from_qmux_bytes(&self.rx[..total]).map_err(Error::Qmi)?;
        self.rx.drain(..total);

        Ok(Some(message))
    }

    fn next_transaction(&mut self) -> u16 {
        self.transaction = self.transaction.wrapping_add(1);
        u16::from(self.transaction)
    }
}

/// One QMI session.
///
/// [`QmiClient::with_session`] hands this to its closure: it carries the
/// transaction id every request of the scope is tagged with and the transport
/// they are sent over. Requests are sent as a generated request type with
/// [`Session::call`], or as a TLV list built by hand with [`Session::request`].
pub struct Session<'a, T: LocalTransport> {
    client: &'a mut QmiClient<T>,
    transaction: u16,
}

impl<T: LocalTransport> Session<'_, T> {
    /// Transaction id every request of this session is tagged with.
    pub fn transaction_id(&self) -> u16 {
        self.transaction
    }

    /// Send one request and decode its response.
    ///
    /// The service and message id come from the request type. A response the
    /// modem rejected reads as the protocol error it was answered with.
    pub async fn call<R: Request>(&mut self, request: R) -> Result<R::Response, Error<T::Error>> {
        let tlvs = request.encode().map_err(Error::Qmi)?;
        let response = self.request(R::SERVICE, R::MESSAGE_ID, tlvs).await?;

        <R::Response as Response>::decode(&response.tlvs).map_err(Error::Qmi)
    }

    /// Send a TLV list built by hand and wait for the response message.
    ///
    /// The message arrives as it was sent: decoding the answer of the request
    /// is what turns its `Result` TLV into data or an error.
    async fn request(
        &mut self,
        service: u16,
        message_id: u16,
        tlvs: Vec<TlvBuf>,
    ) -> Result<Message, Error<T::Error>> {
        self.client
            .exchange(self.transaction, service, message_id, tlvs)
            .await
    }
}

impl<T> LocalUim for QmiClient<T>
where
    T: LocalTransport,
    T::Error: core::error::Error + 'static,
{
    type Error = Error<T::Error>;

    async fn state(&mut self) -> Result<BTreeMap<u8, SlotState>, Self::Error> {
        self.with_session(async |session| {
            let response = session
                .call(uim::messages::get_card_status::Request::default())
                .await?;

            let cards = response
                .card_status
                .as_ref()
                .map(|status| status.cards.as_slice())
                .unwrap_or_default();

            // ATR and IMEI are not part of Get Card Status.
            // Both are best effort.
            let atrs = session
                .call(uim::messages::get_slot_status::Request::default())
                .await
                .ok()
                .map(|response| {
                    let mut atrs = BTreeMap::new();

                    for (index, slot) in response
                        .physical_slot_information
                        .unwrap_or_default()
                        .into_iter()
                        .enumerate()
                    {
                        let number = u8::try_from(index + 1).unwrap_or(u8::MAX);

                        if !slot.atr_value.is_empty() {
                            atrs.insert(number, slot.atr_value);
                        }
                    }

                    atrs
                })
                .unwrap_or_default();

            let imei = session
                .call(dms::messages::get_ids::Request::default())
                .await
                .ok()
                .and_then(|response| response.imei);

            let mut states = BTreeMap::new();

            for (index, card) in cards.iter().enumerate() {
                let slot = u8::try_from(index + 1).unwrap_or(u8::MAX);

                states.insert(
                    slot,
                    SlotState {
                        imei: imei.clone(),
                        atr: atrs.get(&slot).cloned(),
                        present: card.card_state != CARD_STATE_ABSENT,
                        ready: card
                            .applications
                            .iter()
                            .any(|app| app.state == CARD_APPLICATION_STATE_READY),
                        extensions: BTreeMap::new(),
                    },
                );
            }

            Ok(states)
        })
        .await
    }

    async fn reset(&mut self, slot: u8) -> Result<Vec<u8>, Self::Error> {
        self.with_session(async |session| {
            session
                .call(uim::messages::reset::Request::default())
                .await?;

            let response = session
                .call(uim::messages::get_slot_status::Request::default())
                .await?;

            response
                .physical_slot_information
                .unwrap_or_default()
                .into_iter()
                .nth(usize::from(slot).saturating_sub(1))
                .map(|slot| slot.atr_value)
                .filter(|atr| !atr.is_empty())
                .ok_or(Error::Missing("atr"))
        })
        .await
    }

    async fn open_channel(&mut self, slot: u8, aid: &[u8]) -> Result<u8, Self::Error> {
        self.with_session(async |session| {
            let response = session
                .call(uim::messages::open_logical_channel::Request {
                    slot,
                    aid: Some(aid.to_vec()),
                    file_control_information: None,
                })
                .await?;

            response.channel_id.ok_or(Error::Missing("channel_id"))
        })
        .await
    }

    async fn transmit(
        &mut self,
        slot: u8,
        channel: u8,
        apdu: &[u8],
    ) -> Result<Vec<u8>, Self::Error> {
        self.with_session(async |session| {
            let response = session
                .call(uim::messages::send_apdu::Request {
                    slot,
                    apdu: apdu.to_vec(),
                    channel_id: Some(channel),
                    procedure_bytes: None,
                })
                .await?;

            response
                .apdu_response
                .filter(|response| !response.is_empty())
                .ok_or(Error::Missing("apdu_response"))
        })
        .await
    }

    async fn close_channel(&mut self, slot: u8, channel: u8) -> Result<(), Self::Error> {
        self.with_session(async |session| {
            session
                .call(uim::messages::logical_channel::Request {
                    slot,
                    aid: None,
                    channel_id: Some(channel),
                    file_control_information: None,
                    terminate_application: None,
                })
                .await?;

            Ok(())
        })
        .await
    }
}
