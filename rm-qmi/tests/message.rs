use rm_qmi::{Error, Message, QMUX_MARKER, QRTR_MARKER, Service, TlvBuf};

/// CTL Get Version Info response captured from a modem: the service list
/// reports WDS 1.44.
const VERSION_INFO: &[u8] = &[
    0x01, 0x1b, 0x00, // marker, length 27
    0x00, 0x00, 0x00, // transport flags, service CTL, client 0
    0x00, // QMI flags
    0x01, // transaction id
    0x21, 0x00, // message id 33
    0x10, 0x00, // TLV length 16
    0x02, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, // Result: success
    0x01, 0x06, 0x00, 0x01, 0x01, 0x01, 0x00, 0x2c, 0x00, // service list: WDS 1.44
];

#[test]
fn decode_qmux_version_info() {
    let message = Message::from_qmux_bytes(VERSION_INFO).unwrap();

    assert_eq!(message.service, Service::Ctl as u16);
    assert_eq!(message.client_id, 0);
    assert_eq!(message.transaction_id, 1);
    assert_eq!(message.message_id, 33);
    assert!(message.is_control());
    assert_eq!(message.tlvs.len(), 2);
    assert_eq!(message.find_tlv(0x02).unwrap().value, &[0, 0, 0, 0]);
    assert_eq!(
        message.find_tlv(0x01).unwrap().value,
        &[1, 1, 1, 0, 0x2c, 0]
    );
}

#[test]
fn qmux_version_info_round_trip() {
    let message = Message::from_qmux_bytes(VERSION_INFO).unwrap();
    assert_eq!(message.to_qmux_bytes().as_slice(), VERSION_INFO);
}

#[test]
fn qmux_uses_single_byte_transaction_id() {
    let message = Message::new(Service::Dms as u16, 3, 0x1234, 0x0021).with_tlvs(vec![TlvBuf {
        id: 1,
        value: vec![0xaa],
    }]);
    let bytes = message.to_qmux_bytes();

    assert_eq!(bytes[0], QMUX_MARKER);
    assert_eq!(&bytes[1..3], &[0x10, 0x00]); // body 11 bytes + 5
    assert_eq!(bytes[3], 0x00); // transport flags
    assert_eq!(bytes[4], 0x02); // service DMS
    assert_eq!(bytes[5], 3); // client id
    // Non-control services carry a 16-bit transaction id.
    assert_eq!(&bytes[7..9], &[0x34, 0x12]);

    let decoded = Message::from_qmux_bytes(&bytes).unwrap();
    assert_eq!(decoded, message);
}

#[test]
fn qrtr_uses_16_bit_service_id() {
    let message = Message::new(Service::Ssc as u16, 7, 0x1234, 0x0020).with_tlvs(vec![TlvBuf {
        id: 1,
        value: vec![0xaa],
    }]);
    let bytes = message.to_qmux_bytes();

    assert_eq!(bytes[0], QRTR_MARKER);
    assert_eq!(&bytes[3..5], &[0x90, 0x01]); // service SSC 0x0190
    assert_eq!(bytes[5], 7);

    let decoded = Message::from_qmux_bytes(&bytes).unwrap();
    assert_eq!(decoded, message);
    assert_eq!(decoded.service, 0x0190);
}

#[test]
fn truncated_frames_are_rejected() {
    assert_eq!(Message::from_qmux_bytes(&[]), Err(Error::Truncated));
    assert_eq!(
        Message::from_qmux_bytes(&[0x01, 0x00, 0x00]),
        Err(Error::Truncated)
    );
    assert_eq!(
        Message::from_qmux_bytes(&[0x01, 0x04, 0x00, 0x00, 0x00, 0x00]),
        Err(Error::Truncated)
    );
}

#[test]
fn unknown_marker_is_rejected() {
    assert_eq!(
        Message::from_qmux_bytes(&[0x09, 0x00, 0x00, 0x00, 0x00, 0x00]),
        Err(Error::InvalidLength)
    );
}
