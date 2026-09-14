//! `UsbBusExt::request` picks a device with the client's own filter and opens
//! the client on it, with no hardware involved.

use core::convert::Infallible;

use rm_usb_core::{
    DeviceFilter, DeviceInfo, LocalUsbBus, LocalUsbClient, LocalUsbDevice, LocalUsbTransfer,
    SetupPacket, Transfer, TransferBuffer, TransferResult, TransferType, UsbBusExt,
};

/// The device the test client claims to drive.
const FILTER: [DeviceFilter; 1] = [DeviceFilter::new(0x2c7c, 0x0125)];

/// Bus that records the filters it was asked for and always finds a device.
#[derive(Default)]
struct TestBus {
    requested: Vec<DeviceFilter>,
}

impl LocalUsbBus for TestBus {
    type Error = Infallible;
    type Device = TestDevice;

    async fn devices(&mut self) -> Result<Vec<Self::Device>, Self::Error> {
        Ok(Vec::new())
    }

    async fn request_device(
        &mut self,
        filters: &[DeviceFilter],
    ) -> Result<Self::Device, Self::Error> {
        self.requested = filters.to_vec();
        Ok(TestDevice)
    }
}

struct TestDevice;

impl LocalUsbDevice for TestDevice {
    type Error = Infallible;
    type Transfer = TestTransfer;

    fn info(&self) -> DeviceInfo {
        DeviceInfo {
            product_id: 0x0125,
            vendor_id: 0x2c7c,
            usb_version: 0x0200,
            class: 0,
            subclass: 0,
            protocol: 0,
            interfaces: Vec::new(),
        }
    }

    async fn open(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn select_configuration(&mut self, _configuration: u8) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn claim_interface(&mut self, _interface: u8) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn release_interface(&mut self, _interface: u8) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn set_alternate_setting(
        &mut self,
        _interface: u8,
        _alternate_setting: u8,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn reset(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn transfer(&mut self) -> Self::Transfer {
        TestTransfer
    }
}

/// Nothing in this test issues a transfer.
struct TestTransfer;

impl LocalUsbTransfer for TestTransfer {
    type Error = Infallible;

    async fn control_transfer(
        &mut self,
        _setup: SetupPacket,
        _buffer: TransferBuffer<'_>,
    ) -> Result<TransferResult, Self::Error> {
        unimplemented!("the test client does not transfer")
    }

    async fn transfer(&mut self, _transfer: Transfer<'_>) -> Result<TransferResult, Self::Error> {
        unimplemented!("the test client does not transfer")
    }

    fn is_stall(_error: &Self::Error) -> bool {
        false
    }

    async fn clear_halt(
        &mut self,
        _transfer_type: TransferType,
        _endpoint: u8,
    ) -> Result<(), Self::Error> {
        unimplemented!("the test client does not transfer")
    }
}

/// Client that keeps the device it was opened on.
struct TestClient {
    device: TestDevice,
}

impl LocalUsbClient for TestClient {
    type Device = TestDevice;
    type Error = Infallible;

    const PROTOCOL: &'static str = "test";
    const CLIENT_FILTER: &'static [DeviceFilter] = &FILTER;

    async fn open(device: Self::Device) -> Result<Self, Self::Error> {
        Ok(Self { device })
    }
}

#[test]
fn request_opens_the_client_on_a_matching_device() {
    let mut bus = TestBus::default();
    let client = futures::executor::block_on(bus.request::<TestClient>()).unwrap();

    assert_eq!(bus.requested, FILTER);
    assert_eq!(client.device.info().product_id, 0x0125);
}
