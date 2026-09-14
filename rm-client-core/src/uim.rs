use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SlotState {
    pub imei: Option<String>,
    pub atr: Option<Vec<u8>>,
    pub present: bool,
    pub ready: bool,
    pub extensions: BTreeMap<String, Value>,
}

#[trait_variant::make(Uim: Send)]
pub trait LocalUim {
    type Error: core::error::Error;

    async fn state(&mut self) -> Result<BTreeMap<u8, SlotState>, Self::Error>;

    async fn reset(&mut self, slot: u8) -> Result<Vec<u8>, Self::Error>;

    async fn open_channel(&mut self, slot: u8, aid: &[u8]) -> Result<u8, Self::Error>;

    async fn transmit(
        &mut self,
        slot: u8,
        channel: u8,
        apdu: &[u8],
    ) -> Result<Vec<u8>, Self::Error>;

    async fn close_channel(&mut self, slot: u8, channel: u8) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    /// Unsigned integer field.
    Uint32(u32),
    /// Signed integer field.
    Int32(i32),
    /// Text field.
    String(String),
    /// Floating point field.
    F32(f32),
}
