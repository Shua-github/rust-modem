//! Read the card state over QRTR instead of USB.
//!
//! The socket is the modem's, not the cable's: on a Linux host the QRTR bus is
//! the kernel's, and on Android it is reachable from the `shell` user, which is
//! how an app gets one — a helper opens the socket and hands over the
//! descriptor, and [`QrtrTransport::from_fd`] picks it up from there.
//!
//! ```bash
//! cargo run -p rm-qmi --features qrtr --example qrtr_uim_state
//! ```

#[cfg(any(target_os = "linux", target_os = "android"))]
fn main() {
    use rm_client_core::LocalUim;
    use rm_qmi::QmiClient;
    use rm_qmi::QrtrTransport;

    futures::executor::block_on(async {
        let transport = QrtrTransport::open_socket().unwrap();
        let mut client = QmiClient::new(transport);

        let state = client.state().await.unwrap();
        println!(
            "uim state: {}",
            serde_json::to_string_pretty(&state).unwrap()
        );
    });
}

/// The bus is the kernel's, and only the kernel's: there is nothing to open
/// anywhere else.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn main() {
    eprintln!("QRTR is only reachable on Linux and Android");
}
