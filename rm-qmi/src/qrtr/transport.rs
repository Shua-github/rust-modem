//! QMI over QRTR: the ports on the bus, and the datagrams between them.
//!
//! The bus itself is behind [`LocalQrtrSocket`], so this is the half of QRTR
//! that does not depend on who opened the socket: it learns where a service
//! lives, addresses it, and hands the client above one framing to read,
//! whichever process the datagrams actually pass through.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;

use crate::Message;
use crate::message::FRAME_HEADER_LEN;
use crate::transport::{LocalTransport, Multiplexing};

use super::socket::{LocalQrtrSocket, QrtrAddress};

/// Node a lookup is addressed to when the local one is not usable.
const QRTR_NODE_BCAST: u32 = 0xffff_ffff;

/// Port the name service and the other bus control packets live on.
const QRTR_PORT_CTRL: u32 = 0xffff_fffe;

/// `QRTR_TYPE_NEW_LOOKUP`: asking the bus where a service is.
const QRTR_TYPE_NEW_LOOKUP: u32 = 10;

/// `QRTR_TYPE_NEW_SERVER`: the bus answering where a service is.
const QRTR_TYPE_NEW_SERVER: u32 = 4;

/// Biggest datagram QRTR carries; the kernel refuses anything longer.
const MAX_DATAGRAM: usize = 65_535;

/// How long to wait for the bus to answer, and for a service to reply.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

/// A QRTR control packet, as the bus sends and receives them.
///
/// The four words after the command are the `server` member of the kernel's
/// union, which is the shape both lookups and their answers use.
#[derive(Debug, Clone, Copy, Default)]
struct CtrlPacket {
    cmd: u32,
    service: u32,
    instance: u32,
    node: u32,
    port: u32,
}

impl CtrlPacket {
    const LEN: usize = 20;

    fn to_bytes(self) -> [u8; Self::LEN] {
        let mut bytes = [0u8; Self::LEN];
        for (index, word) in [self.cmd, self.service, self.instance, self.node, self.port]
            .into_iter()
            .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }

        bytes
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        let word = |index: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                bytes.get(index * 4..index * 4 + 4)?.try_into().ok()?,
            ))
        };

        Some(Self {
            cmd: word(0)?,
            service: word(1)?,
            instance: word(2)?,
            node: word(3)?,
            port: word(4)?,
        })
    }
}

/// Errors the QRTR transport reports.
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    /// The socket under the transport failed.
    #[error("qrtr socket: {0}")]
    Socket(#[source] E),
    /// Nothing answered within the timeout.
    #[error("qrtr did not answer within {0:?}")]
    Timeout(Duration),
    /// The bus has no service with that id.
    #[error("no QRTR service {service:#06x} on the bus")]
    NoService { service: u16 },
    /// A frame the transport cannot put into one datagram.
    #[error("qrtr frame of {0} bytes does not fit a datagram")]
    FrameTooLong(usize),
    /// A frame handed in without the transport header it should have.
    #[error("qrtr frame of {0} bytes is too short to carry what it has to")]
    ShortFrame(usize),
}

/// A modem reached over the QRTR bus.
pub struct QrtrTransport<S: LocalQrtrSocket> {
    socket: S,
    /// Ports resolved so far, by QMI service.
    services: BTreeMap<u16, QrtrAddress>,
    timeout: Duration,
    /// The datagram being handed out, so a frame larger than the caller's
    /// buffer still arrives whole.
    pending: Vec<u8>,
    pending_offset: usize,
    /// Scratch space for the next datagram.
    datagram: Vec<u8>,
}

impl<S> QrtrTransport<S>
where
    S: LocalQrtrSocket,
    S::Error: 'static,
{
    /// Wrap a socket to the bus.
    pub fn new(socket: S) -> Self {
        Self {
            socket,
            services: BTreeMap::new(),
            timeout: DEFAULT_TIMEOUT,
            pending: Vec::new(),
            pending_offset: 0,
            datagram: alloc::vec![0u8; MAX_DATAGRAM],
        }
    }

    /// Where a QMI `service` lives on the bus.
    ///
    /// Control is not a service the bus publishes: the messages that belong to
    /// it are answered by the link itself, so a lookup that comes back empty is
    /// the answer.
    async fn resolve(&mut self, service: u16) -> Result<QrtrAddress, Error<S::Error>> {
        if let Some(port) = self.services.get(&service) {
            return Ok(*port);
        }

        if let Some(port) = self.looked_up(service).await? {
            self.services.insert(service, port);
            return Ok(port);
        }

        Err(Error::NoService { service })
    }

    /// The service a message from `address` belongs to.
    ///
    /// The bus puts no service in what it carries, so the port it came from is
    /// the only thing that says which service answered.
    fn service_of(&self, address: QrtrAddress) -> Option<u16> {
        self.services
            .iter()
            .find(|(_, known)| **known == address)
            .map(|(service, _)| *service)
    }

    /// Ask the bus where `service` is; `None` when the bus does not list it.
    async fn looked_up(&mut self, service: u16) -> Result<Option<QrtrAddress>, Error<S::Error>> {
        let lookup = CtrlPacket {
            cmd: QRTR_TYPE_NEW_LOOKUP,
            service: u32::from(service),
            ..CtrlPacket::default()
        };

        let node = self.lookup_node().await?;
        let to = QrtrAddress {
            node,
            port: QRTR_PORT_CTRL,
        };

        self.socket
            .send(to, &lookup.to_bytes())
            .await
            .map_err(Error::Socket)?;

        let mut found = None;

        loop {
            let mut buffer = core::mem::take(&mut self.datagram);
            let arrived = self.socket.receive(&mut buffer, self.timeout).await;
            self.datagram = buffer;

            let Some((length, from)) = arrived.map_err(Error::Socket)? else {
                // Every list ends with a record naming nothing at all. Not
                // seeing it means the bus simply did not answer.
                return Err(Error::Timeout(self.timeout));
            };

            // Only the name service answers a lookup, and only with a server
            // record; anything else on this socket is somebody else's.
            if from.port != QRTR_PORT_CTRL {
                continue;
            }

            let Some(answer) = CtrlPacket::parse(&self.datagram[..length]) else {
                continue;
            };

            // The record that names nothing at all is the bus saying the
            // service is not there. It is checked before the service is,
            // because a list asked for service zero — which the bus reads as
            // "no filter" — ends with one of these too. Reading all the way to
            // it is what keeps it, and the rest of the list, out of the queue
            // `read` hands the client.
            if answer.cmd == QRTR_TYPE_NEW_SERVER && answer.node == 0 && answer.port == 0 {
                return Ok(found);
            }

            if answer.cmd == QRTR_TYPE_NEW_SERVER
                && answer.service == u32::from(service)
                && found.is_none()
            {
                found = Some(QrtrAddress {
                    node: answer.node,
                    port: answer.port,
                });
            }
        }
    }

    /// Node a lookup is addressed to: our own, or the broadcast node when the
    /// socket does not have a usable one.
    async fn lookup_node(&mut self) -> Result<u32, Error<S::Error>> {
        let node = self.socket.address().await.map_err(Error::Socket)?.node;

        Ok(if node == 0 || node == QRTR_NODE_BCAST {
            QRTR_NODE_BCAST
        } else {
            node
        })
    }
}

impl<S> LocalTransport for QrtrTransport<S>
where
    S: LocalQrtrSocket,
    S::Error: 'static,
{
    type Error = Error<S::Error>;

    fn multiplexing(&self) -> Multiplexing {
        Multiplexing::Qrtr
    }

    async fn write(&mut self, service: u16, frame: &[u8]) -> Result<(), Error<S::Error>> {
        // What the bus carries is the QMI message alone: the marker and the
        // header that name the service are what the port underneath them
        // stands for. libqmi does the same, see its `qmi-endpoint-qrtr.c`.
        let body = frame
            .get(FRAME_HEADER_LEN..)
            .ok_or(Error::ShortFrame(frame.len()))?;

        if body.len() > MAX_DATAGRAM {
            return Err(Error::FrameTooLong(body.len()));
        }

        let to = self.resolve(service).await?;

        self.socket.send(to, body).await.map_err(Error::Socket)
    }

    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error<S::Error>> {
        if self.pending_offset < self.pending.len() {
            let available = &self.pending[self.pending_offset..];
            let copied = available.len().min(out.len());
            out[..copied].copy_from_slice(&available[..copied]);
            self.pending_offset += copied;

            return Ok(copied);
        }

        let (length, service) = loop {
            let mut buffer = core::mem::take(&mut self.datagram);
            let arrived = self.socket.receive(&mut buffer, self.timeout).await;
            self.datagram = buffer;

            let Some((length, from)) = arrived.map_err(Error::Socket)? else {
                return Err(Error::Timeout(self.timeout));
            };

            // Records from the bus itself are not QMI frames; skip them rather
            // than hand one to the client to decode.
            if from.port == QRTR_PORT_CTRL {
                continue;
            }

            match self.service_of(from) {
                Some(service) => break (length, service),
                // A port no service was looked up for has nothing to do with
                // anything asked from here.
                None => continue,
            }
        };

        // The client above speaks one framing whichever link it is on, so put
        // the header the bus left out back on: the service is the port this
        // came from, and the client id is nobody's business here.
        self.pending.clear();
        self.pending
            .extend_from_slice(&Message::frame_header(service, 0, length));
        self.pending.extend_from_slice(&self.datagram[..length]);

        let copied = self.pending.len().min(out.len());
        out[..copied].copy_from_slice(&self.pending[..copied]);
        self.pending_offset = copied;

        Ok(copied)
    }
}
