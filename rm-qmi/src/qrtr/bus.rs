//! The socket on the bus itself.
//!
//! This is what a process that is allowed to open one uses directly: the
//! kernel's `AF_QIPCRTR` socket, bound on its first send, with the address
//! family named in every address that goes out. Everything about the protocol
//! is above it; what is left here is datagrams.

/// Sockets and file descriptors are `std`; this module brings `std` into an
/// otherwise `no_std` crate, and nothing outside it needs that.
extern crate std;

use core::mem::size_of;
use core::time::Duration;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Instant;

use super::socket::{LocalQrtrSocket, QrtrAddress};
use super::transport;
use super::transport::QrtrTransport;

/// `AF_QIPCRTR` as upstream defines it (`linux/qrtr.h`).
const AF_QIPCRTR: u16 = 43;

/// The number Qualcomm's own kernels registered QRTR under, which many Android
/// devices still use: they were built before the upstream allocation landed.
const AF_QIPCRTR_QCOM: u16 = 42;

/// The families a QRTR socket may be on, upstream first.
///
/// Which one a device uses cannot be asked, only tried.
const AF_QIPCRTR_CANDIDATES: [u16; 2] = [AF_QIPCRTR, AF_QIPCRTR_QCOM];

/// Biggest datagram QRTR carries; the kernel refuses anything longer.
const MAX_DATAGRAM: usize = 65_535;

/// One address on the QRTR bus, as the kernel writes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SockAddrQrtr {
    sq_family: libc::sa_family_t,
    sq_node: u32,
    sq_port: u32,
}

/// Errors a socket on the bus reports.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The socket could not be used at all.
    #[error("qrtr socket: {0}")]
    Socket(#[source] io::Error),
    /// The descriptor handed in is not a QRTR socket.
    #[error("file descriptor is not a QRTR socket (address family {0})")]
    NotQrtr(libc::sa_family_t),
}

/// A datagram socket on the QRTR bus.
pub struct BusSocket {
    socket: OwnedFd,
    /// Address family the socket is on, which every address sent on it has to
    /// name as well.
    family: u16,
    /// Node this socket is bound to, which is what lookups are addressed to.
    node: u32,
}

impl BusSocket {
    /// Open the QRTR socket of this process.
    ///
    /// Only a process the bus lets in — on Android that means `shell`, not an
    /// app — can do this; anywhere else, open the socket elsewhere and hand it
    /// to [`Self::from_fd`].
    pub fn open() -> Result<Self, Error> {
        let mut last = None;

        for family in AF_QIPCRTR_CANDIDATES {
            let socket = unsafe {
                libc::socket(
                    libc::c_int::from(family),
                    libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
                    0,
                )
            };

            if socket >= 0 {
                return Self::from_fd(socket);
            }

            last = Some(io::Error::last_os_error());
        }

        Err(Error::Socket(last.unwrap_or_else(|| {
            io::Error::from_raw_os_error(libc::EAFNOSUPPORT)
        })))
    }

    /// Wrap a QRTR socket that somebody else opened.
    ///
    /// Takes ownership of `fd`: the caller gives the descriptor up, and the
    /// socket closes it when it is dropped.
    ///
    /// The kernel binds the socket on its first send, so an unbound one is
    /// fine: the first lookup is what gives it its port.
    pub fn from_fd(fd: i32) -> Result<Self, Error> {
        if fd < 0 {
            return Err(Error::Socket(io::Error::from_raw_os_error(libc::EBADF)));
        }

        // SAFETY: the caller hands over a descriptor it owns and no longer
        // uses, which is what taking ownership means.
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let local = local_address(&socket)?;

        Ok(Self {
            socket,
            family: local.sq_family,
            node: local.sq_node,
        })
    }
}

impl QrtrTransport<BusSocket> {
    /// Open this process's socket on the bus and drive the modem over it.
    ///
    /// Only a process the bus lets in — on Android that means `shell`, not an
    /// app — gets that far; anywhere else, open the socket elsewhere and hand
    /// it to [`QrtrTransport::from_fd`].
    pub fn open_socket() -> Result<Self, transport::Error<Error>> {
        BusSocket::open()
            .map_err(transport::Error::Socket)
            .map(Self::new)
    }

    /// Drive the modem over a socket that somebody else opened.
    pub fn from_fd(fd: i32) -> Result<Self, transport::Error<Error>> {
        BusSocket::from_fd(fd)
            .map_err(transport::Error::Socket)
            .map(Self::new)
    }
}

impl LocalQrtrSocket for BusSocket {
    type Error = Error;

    async fn address(&mut self) -> Result<QrtrAddress, Error> {
        Ok(QrtrAddress {
            node: self.node,
            // The kernel hands out the port on the first send, so there is
            // none to report: a lookup does not need one.
            port: 0,
        })
    }

    async fn send(&mut self, to: QrtrAddress, datagram: &[u8]) -> Result<(), Error> {
        let address = SockAddrQrtr {
            sq_family: self.family,
            sq_node: to.node,
            sq_port: to.port,
        };

        let written = unsafe {
            libc::sendto(
                self.socket.as_raw_fd(),
                datagram.as_ptr().cast(),
                datagram.len(),
                0,
                (&raw const address).cast(),
                size_of::<SockAddrQrtr>() as libc::socklen_t,
            )
        };

        if written < 0 {
            return Err(Error::Socket(io::Error::last_os_error()));
        }

        Ok(())
    }

    async fn receive(
        &mut self,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<Option<(usize, QrtrAddress)>, Error> {
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }

            let mut poll = libc::pollfd {
                fd: self.socket.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };

            let ready = unsafe { libc::poll(&mut poll, 1, poll_timeout(remaining)) };

            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }

                return Err(Error::Socket(error));
            }

            if ready == 0 {
                continue;
            }

            let mut from = SockAddrQrtr::default();
            let mut length = size_of::<SockAddrQrtr>() as libc::socklen_t;

            let read = unsafe {
                libc::recvfrom(
                    self.socket.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len().min(MAX_DATAGRAM),
                    0,
                    (&mut from as *mut SockAddrQrtr).cast(),
                    &mut length,
                )
            };

            if read < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }

                return Err(Error::Socket(error));
            }

            return Ok(Some((
                read as usize,
                QrtrAddress {
                    node: from.sq_node,
                    port: from.sq_port,
                },
            )));
        }
    }
}

/// The address this socket is bound to, which is where a lookup starts from.
fn local_address(socket: &OwnedFd) -> Result<SockAddrQrtr, Error> {
    let mut address = SockAddrQrtr::default();
    let mut length = size_of::<SockAddrQrtr>() as libc::socklen_t;

    let named = unsafe {
        libc::getsockname(
            socket.as_raw_fd(),
            (&mut address as *mut SockAddrQrtr).cast(),
            &mut length,
        )
    };

    if named < 0 {
        return Err(Error::Socket(io::Error::last_os_error()));
    }

    if !AF_QIPCRTR_CANDIDATES.contains(&address.sq_family) {
        return Err(Error::NotQrtr(address.sq_family));
    }

    Ok(address)
}

/// Milliseconds for `poll`, never zero so the loop can re-check its deadline.
fn poll_timeout(remaining: Duration) -> libc::c_int {
    let milliseconds = remaining.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;

    milliseconds.max(1)
}
