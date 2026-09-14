//! The `Result` TLV every QMI response carries.
//!
//! A response answers with a `Result` TLV holding an error status and an error
//! code, and its remaining TLVs are only meaningful when the status says the
//! request succeeded. A response the modem rejected reads as the protocol
//! error of that TLV, and the TLVs a failure carries are decoded beside it by
//! the generated `ResponseError`.

use alloc::vec::Vec;

use crate::types::common::OperationResult;
use crate::{Endian, Error, TlvBuf, TlvReader, TlvWriter};

/// TLV id of the mandatory `Result` TLV.
pub const TLV_RESULT: u8 = 0x02;

/// Decode the mandatory `Result` TLV of a response.
pub(crate) fn decode_result(tlvs: &[TlvBuf]) -> Result<OperationResult, Error> {
    let mut reader = TlvReader::new(result_value(tlvs)?);

    OperationResult::decode_from(&mut reader)
}

/// The `Result` TLV of a response the modem accepted.
///
/// A status that says otherwise is the protocol error the request was rejected
/// with.
pub(crate) fn check_result(tlvs: &[TlvBuf]) -> Result<OperationResult, Error> {
    let result = decode_result(tlvs)?;

    if result.error_status == 0 {
        Ok(result)
    } else {
        Err(Error::Qmi(result))
    }
}

/// The `Result` TLV of a response the modem accepted.
pub(crate) fn success_tlvs() -> Vec<TlvBuf> {
    let mut writer = TlvWriter::new();
    writer.write_u16(0, Endian::Little);
    writer.write_u16(0, Endian::Little);

    alloc::vec![writer.into_tlv(TLV_RESULT)]
}

fn result_value(tlvs: &[TlvBuf]) -> Result<&[u8], Error> {
    tlvs.iter()
        .find(|tlv| tlv.id == TLV_RESULT)
        .map(|tlv| tlv.value.as_slice())
        .ok_or(Error::MissingTlv { id: TLV_RESULT })
}

/// A `Result` TLV reads as its own summary where the runtime reports one, in
/// [`crate::Error::Qmi`].
///
/// The impl lives here instead of beside the type because
/// `types/common.rs` is generated: `cargo xtask qmi-codegen` rewrites it, and
/// the derives the type needs are emitted from `xtask/src/qmi/emit.rs`.
impl core::fmt::Display for OperationResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "status {}, code {}", self.error_status, self.error_code)
    }
}
