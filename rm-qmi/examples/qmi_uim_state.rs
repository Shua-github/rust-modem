use rm_client_core::LocalUim;
use rm_usb_core::{UsbBus, UsbBusExt};
use rm_usb_nusb::NusbBus;

type Bus = NusbBus;
type Device = <NusbBus as UsbBus>::Device;
type Client = rm_qmi::UsbQmiClient<Device>;

fn main() {
    futures::executor::block_on(async {
        let mut bus = Bus::new();
        let mut client = bus.request::<Client>().await.unwrap();
        let state = client.state().await.unwrap();
        println!(
            "uim state: {}",
            serde_json::to_string_pretty(&state).unwrap()
        );
    });
}
