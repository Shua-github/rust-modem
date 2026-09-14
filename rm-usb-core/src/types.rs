#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionType {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Recipient {
    Device = 0,
    Interface = 1,
    Endpoint = 2,
    Other = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlType {
    Standard = 0,
    Class = 1,
    Vendor = 2,
}

/// USB setup packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SetupPacket {
    pub recipient: Recipient,
    pub control_type: ControlType,
    pub direction: DirectionType,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}
