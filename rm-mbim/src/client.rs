//! MBIM client bound to one CDC MBIM control function, including the shared
//! UIM interface.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use rm_cdc_wdm::CdcWdm;
use rm_client_core::{LocalUim, SlotState, Value};
use rm_usb_core::{DeviceFilter, LocalUsbClient, LocalUsbDevice};

use crate::types::{basic_connect as mbim_basic, ms_uicc_low_level_access as mbim_uicc};
use crate::{Message, MessageType, Request, Response, Uuid};

/// `MbimSubscriberReadyState::Initialized`.
const READY_STATE_INITIALIZED: u32 = 1;
/// `MbimSubscriberReadyState::SimNotInserted`.
const READY_STATE_SIM_NOT_INSERTED: u32 = 2;

/// CDC MBIM (subclass 0x0e) is the interface-level filter for MBIM functions.
const FILTERS: [DeviceFilter; 1] = [DeviceFilter::class(0x02, 0x0e, 0x00)];

/// MBIM's low level access service drives a single card.
const SLOT: u8 = 1;

/// A MBIM client bound to one CDC MBIM control function.
pub struct MbimClient<D: LocalUsbDevice> {
    wdm: CdcWdm<D>,
    transaction: u32,
    rx: Vec<u8>,
}

impl<D: LocalUsbDevice> MbimClient<D> {
    pub fn new(wdm: CdcWdm<D>) -> Self {
        Self {
            wdm,
            transaction: 0,
            rx: Vec::new(),
        }
    }

    pub fn max_control_message_size(&self) -> u16 {
        self.wdm.max_command_size()
    }

    pub fn interface_number(&self) -> u8 {
        self.wdm.interface_number()
    }

    pub fn into_device(self) -> D {
        self.wdm.into_device()
    }

    async fn open_session(&mut self) -> Result<(), Error<D::Error>> {
        let transaction = self.next_transaction();
        self.send(&Message::open(
            transaction,
            u32::from(self.max_control_message_size()),
        ))
        .await?;

        let response = self.wait_for(MessageType::OpenDone, transaction).await?;
        let status = response.status_code().map_err(Error::Mbim)?;
        if status != 0 {
            return Err(Error::Mbim(crate::Error::Status(status)));
        }
        Ok(())
    }

    async fn close_session(&mut self) -> Result<(), Error<D::Error>> {
        let transaction = self.next_transaction();
        self.send(&Message::close(transaction)).await?;

        let response = self.wait_for(MessageType::CloseDone, transaction).await?;
        let status = response.status_code().map_err(Error::Mbim)?;
        if status != 0 {
            return Err(Error::Mbim(crate::Error::Status(status)));
        }
        Ok(())
    }

    pub async fn with_session<T, F>(&mut self, f: F) -> Result<T, Error<D::Error>>
    where
        F: AsyncFnOnce(&mut Session<'_, D>) -> Result<T, Error<D::Error>>,
    {
        self.open_session().await?;
        let transaction = self.next_transaction();
        let result = {
            let mut session = Session {
                client: self,
                transaction,
            };
            f(&mut session).await
        };
        let closed = self.close_session().await;

        match (result, closed) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    async fn send(&mut self, message: &Message) -> Result<(), Error<D::Error>> {
        for fragment in message
            .fragments(self.max_control_message_size())
            .map_err(Error::Mbim)?
        {
            self.wdm.write(&fragment).await.map_err(Error::Wdm)?;
        }
        Ok(())
    }

    async fn wait_for_command(
        &mut self,
        transaction_id: u32,
        service: Uuid,
        cid: u32,
    ) -> Result<Message, Error<D::Error>> {
        loop {
            let response = self.read_complete().await?;
            if response.transaction_id != transaction_id {
                continue;
            }

            match response.message_type {
                MessageType::IndicateStatus => continue,
                MessageType::FunctionError => {
                    let status = response.status_code().map_err(Error::Mbim)?;
                    return Err(Error::Mbim(crate::Error::Status(status)));
                }
                MessageType::CommandDone => {
                    let info = response.command_info().map_err(Error::Mbim)?;
                    if info.service != service || info.cid != cid {
                        continue;
                    }
                    if let Some(status) = info.status.filter(|status| *status != 0) {
                        return Err(Error::Mbim(crate::Error::Status(status)));
                    }
                    return Ok(response);
                }
                _ => continue,
            }
        }
    }

    async fn wait_for(
        &mut self,
        expected_type: MessageType,
        transaction: u32,
    ) -> Result<Message, Error<D::Error>> {
        loop {
            let response = self.read_complete().await?;
            if response.transaction_id != transaction {
                continue;
            }

            if response.message_type == MessageType::FunctionError {
                let status = response.status_code().map_err(Error::Mbim)?;
                return Err(Error::Mbim(crate::Error::Status(status)));
            }
            if response.message_type == expected_type {
                return Ok(response);
            }
        }
    }

    async fn read_complete(&mut self) -> Result<Message, Error<D::Error>> {
        let first = self.read_one().await?;
        if !first.message_type.is_fragmented() {
            return Ok(first);
        }

        let (total, _) = first.fragment().map_err(Error::Mbim)?;
        if total == 1 {
            return Ok(first);
        }

        let mut fragments = Vec::with_capacity(total as usize);
        fragments.push(first);
        while fragments.len() < total as usize {
            fragments.push(self.read_one().await?);
        }
        Message::merge_fragments(&fragments).map_err(Error::Mbim)
    }

    async fn read_one(&mut self) -> Result<Message, Error<D::Error>> {
        loop {
            if self.rx.len() >= 12 {
                let length = usize::try_from(u32::from_le_bytes(self.rx[4..8].try_into().unwrap()))
                    .map_err(|_| Error::Mbim(crate::Error::InvalidLength))?;
                if !(12..=usize::from(self.max_control_message_size())).contains(&length) {
                    return Err(Error::Mbim(crate::Error::InvalidLength));
                }
                if self.rx.len() >= length {
                    let bytes: Vec<u8> = self.rx.drain(..length).collect();
                    return Message::from_bytes(&bytes).map_err(Error::Mbim);
                }
            }

            let mut chunk = [0u8; 4096];
            let read = self.wdm.read(&mut chunk).await.map_err(Error::Wdm)?;
            if read == 0 {
                return Err(Error::Mbim(crate::Error::Truncated));
            }
            self.rx.extend_from_slice(&chunk[..read]);
        }
    }

    fn next_transaction(&mut self) -> u32 {
        self.transaction = self.transaction.wrapping_add(1);
        self.transaction
    }
}

/// One open MBIM session.
///
/// [`MbimClient::with_session`] hands this to its closure: it carries the
/// transaction id every message of the scope is tagged with and the transport
/// they are sent over. Commands are sent as a generated request type with
/// [`Session::call`], or built by hand and sent with [`Session::send`]
/// followed by [`Session::wait_for_command`].
pub struct Session<'a, D: LocalUsbDevice> {
    client: &'a mut MbimClient<D>,
    transaction: u32,
}

impl<D: LocalUsbDevice> Session<'_, D> {
    /// Transaction id every message of this session is tagged with.
    pub fn transaction_id(&self) -> u32 {
        self.transaction
    }

    /// Send one command and decode its response.
    ///
    /// The service, command id and command type come from the request type.
    pub async fn call<T: Request>(&mut self, request: T) -> Result<T::Response, Error<D::Error>> {
        let message = Message::command(
            self.transaction,
            T::SERVICE,
            T::CID,
            T::COMMAND_TYPE,
            &request.encode(),
        );
        self.send(&message).await?;

        let response = self.wait_for_command(T::SERVICE, T::CID).await?;
        let info = response.command_info().map_err(Error::Mbim)?;

        <T::Response as Response>::decode(info.information_buffer).map_err(Error::Mbim)
    }

    async fn send(&mut self, message: &Message) -> Result<(), Error<D::Error>> {
        self.client.send(message).await
    }

    async fn wait_for_command(
        &mut self,
        service: Uuid,
        cid: u32,
    ) -> Result<Message, Error<D::Error>> {
        self.client
            .wait_for_command(self.transaction, service, cid)
            .await
    }
}

/// Errors from a MBIM client, including the underlying WDM transport.
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    #[error("WDM transport error: {0}")]
    Wdm(
        #[source]
        #[from]
        rm_cdc_wdm::Error<E>,
    ),
    #[error("MBIM protocol error: {0}")]
    Mbim(
        #[source]
        #[from]
        crate::Error,
    ),
    #[error("Invalid slot")]
    InvalidSlot,
    #[error("No data")]
    NoData,
    #[error("Channel too large")]
    ChannelTooLarge,
}

impl<B> LocalUsbClient for MbimClient<B>
where
    B: LocalUsbDevice,
    B::Error: core::error::Error + 'static,
{
    type Device = B;
    type Error = Error<B::Error>;

    const PROTOCOL: &str = "mbim";

    const CLIENT_FILTER: &[DeviceFilter] = &FILTERS;

    async fn open(device: B) -> Result<Self, Self::Error> {
        let wdm = CdcWdm::open_with(device, |config, info| {
            crate::find_mbim_interface(config, info)
        })
        .await
        .map_err(Error::Wdm)?;

        Ok(MbimClient::new(wdm))
    }
}

impl<D> LocalUim for MbimClient<D>
where
    D: LocalUsbDevice,
    D::Error: core::error::Error + 'static,
{
    type Error = Error<D::Error>;

    async fn state(&mut self) -> Result<BTreeMap<u8, SlotState>, Self::Error> {
        // The three probes share one session and its transaction id.
        self.with_session(async |session| {
            let response = session
                .call(mbim_basic::subscriber_ready_status::QueryRequest::default())
                .await?;

            let mut extensions = BTreeMap::new();
            extensions.insert(
                "iccid".to_owned(),
                Value::String(response.sim_icc_id.clone()),
            );

            let atr = query_atr(session).await.ok();
            // MBIM has no IMEI command; modems report it as `DeviceId`.
            let imei = query_imei(session).await.ok();

            Ok(BTreeMap::from([(
                SLOT,
                SlotState {
                    imei,
                    atr,
                    present: response.ready_state != READY_STATE_SIM_NOT_INSERTED,
                    ready: response.ready_state == READY_STATE_INITIALIZED,
                    extensions,
                },
            )]))
        })
        .await
    }

    async fn reset(&mut self, slot: u8) -> Result<Vec<u8>, Self::Error> {
        if slot != SLOT {
            return Err(Error::InvalidSlot);
        }
        self.with_session(async |session| {
            session
                .call(mbim_uicc::reset::SetRequest::default())
                .await?;
            query_atr(session).await
        })
        .await
    }

    async fn open_channel(&mut self, slot: u8, aid: &[u8]) -> Result<u8, Self::Error> {
        if slot != SLOT {
            return Err(Error::InvalidSlot);
        }
        self.with_session(async |session| {
            let request = mbim_uicc::open_channel::SetRequest {
                app_id: aid.to_vec(),
                select_p2_arg: 0,
                channel_group: 0,
            };
            let response = session.call(request).await?;

            u8::try_from(response.channel).map_err(|_| Error::ChannelTooLarge)
        })
        .await
    }

    async fn transmit(
        &mut self,
        slot: u8,
        channel: u8,
        apdu: &[u8],
    ) -> Result<Vec<u8>, Self::Error> {
        if slot != SLOT {
            return Err(Error::InvalidSlot);
        }
        self.with_session(async |session| {
            let request = mbim_uicc::a_p_d_u::SetRequest {
                channel: u32::from(channel),
                secure_messaging: 0,
                class_byte_type: 0,
                command: apdu.to_vec(),
            };
            let response = session.call(request).await?;

            Ok(response.response)
        })
        .await
    }

    async fn close_channel(&mut self, slot: u8, channel: u8) -> Result<(), Self::Error> {
        if slot != SLOT {
            return Err(Error::InvalidSlot);
        }
        self.with_session(async |session| {
            session
                .call(mbim_uicc::close_channel::SetRequest {
                    channel: u32::from(channel),
                    channel_group: 0,
                })
                .await?;

            Ok(())
        })
        .await
    }
}

/// Read the modem's `DeviceId`, which MBIM modems report the IMEI in.
async fn query_imei<D>(session: &mut Session<'_, D>) -> Result<String, Error<D::Error>>
where
    D: LocalUsbDevice,
    D::Error: core::error::Error + 'static,
{
    let response = session
        .call(mbim_basic::device_caps::QueryRequest::default())
        .await?;

    if response.device_id.is_empty() {
        return Err(Error::NoData);
    }

    Ok(response.device_id)
}

async fn query_atr<D>(session: &mut Session<'_, D>) -> Result<Vec<u8>, Error<D::Error>>
where
    D: LocalUsbDevice,
    D::Error: core::fmt::Debug,
{
    let response = session
        .call(mbim_uicc::a_t_r::QueryRequest::default())
        .await?;
    if response.atr.is_empty() {
        return Err(Error::NoData);
    }
    Ok(response.atr)
}
