use rm_qmi::{Endian, Error, TlvReader, TlvWriter, tlvs};

#[test]
fn iterate_tlvs() {
    let body = [0x02, 0x02, 0x00, 0xaa, 0xbb, 0x01, 0x01, 0x00, 0x2c];
    let list: Vec<_> = tlvs(&body).collect();

    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, 0x02);
    assert_eq!(list[0].value, &[0xaa, 0xbb]);
    assert_eq!(list[1].id, 0x01);
    assert_eq!(list[1].value, &[0x2c]);
}

#[test]
fn truncated_tlv_stops_iteration() {
    assert_eq!(tlvs(&[0x02, 0x04, 0x00, 0xaa]).count(), 0);
    assert_eq!(tlvs(&[0x02, 0x04]).count(), 0);
    assert_eq!(tlvs(&[]).count(), 0);
}

#[test]
fn scalar_round_trip() {
    let mut writer = TlvWriter::new();
    writer.write_u8(0x12);
    writer.write_i8(-2);
    writer.write_u16(0x1234, Endian::Little);
    writer.write_u16(0x1234, Endian::Big);
    writer.write_i32(-5, Endian::Little);
    writer.write_u64(0x0102_0304_0506_0708, Endian::Little);
    writer.write_f32(1.5, Endian::Little);
    writer.write_f64(-2.25, Endian::Big);
    let tlv = writer.into_tlv(0x10);
    assert_eq!(tlv.id, 0x10);

    let mut reader = TlvReader::new(&tlv.value);
    assert_eq!(reader.read_u8().unwrap(), 0x12);
    assert_eq!(reader.read_i8().unwrap(), -2);
    assert_eq!(reader.read_u16(Endian::Little).unwrap(), 0x1234);
    assert_eq!(reader.read_u16(Endian::Big).unwrap(), 0x1234);
    assert_eq!(reader.read_i32(Endian::Little).unwrap(), -5);
    assert_eq!(
        reader.read_u64(Endian::Little).unwrap(),
        0x0102_0304_0506_0708
    );
    assert_eq!(reader.read_f32(Endian::Little).unwrap(), 1.5);
    assert_eq!(reader.read_f64(Endian::Big).unwrap(), -2.25);
    reader.finish(0x10).unwrap();
}

#[test]
fn sized_uint_round_trip() {
    let mut writer = TlvWriter::new();
    writer.write_uint(0x1234, 2, Endian::Little).unwrap();
    writer.write_uint(0x1234, 2, Endian::Big).unwrap();
    writer.write_uint(0x1234, 4, Endian::Big).unwrap();
    let tlv = writer.into_tlv(0x01);
    assert_eq!(
        tlv.value.as_slice(),
        &[0x34, 0x12, 0x12, 0x34, 0x00, 0x00, 0x12, 0x34]
    );

    let mut reader = TlvReader::new(&tlv.value);
    assert_eq!(reader.read_uint(2, Endian::Little).unwrap(), 0x1234);
    assert_eq!(reader.read_uint(2, Endian::Big).unwrap(), 0x1234);
    assert_eq!(reader.read_uint(4, Endian::Big).unwrap(), 0x1234);
    reader.finish(0x01).unwrap();
}

#[test]
fn string_prefix_round_trip() {
    let mut writer = TlvWriter::new();
    writer.write_string("hello", 1, None).unwrap();
    writer.write_string("world", 2, None).unwrap();
    writer.write_string("plain", 0, None).unwrap();
    let tlv = writer.into_tlv(0x01);
    assert_eq!(tlv.value.as_slice(), b"\x05hello\x05\x00worldplain");

    let mut reader = TlvReader::new(&tlv.value);
    assert_eq!(reader.read_string(1, 0).unwrap(), "hello");
    assert_eq!(reader.read_string(2, 0).unwrap(), "world");
    assert_eq!(reader.read_string(0, 0).unwrap(), "plain");
    reader.finish(0x01).unwrap();
}

#[test]
fn fixed_string_is_padded_and_trimmed() {
    let mut writer = TlvWriter::new();
    writer.write_string("abc", 0, Some(6)).unwrap();
    let tlv = writer.into_tlv(0x01);
    assert_eq!(tlv.value.as_slice(), b"abc\0\0\0");

    let mut reader = TlvReader::new(&tlv.value);
    assert_eq!(reader.read_fixed_string(6).unwrap(), "abc");
    reader.finish(0x01).unwrap();
}

#[test]
fn too_long_string_is_rejected() {
    let mut writer = TlvWriter::new();
    assert_eq!(
        writer.write_string("abcdef", 0, Some(3)),
        Err(Error::TooLong)
    );
    assert_eq!(
        writer.write_string("abcdef", 9, None),
        Err(Error::InvalidLength)
    );
}

#[test]
fn read_string_truncates_to_max_size() {
    let mut reader = TlvReader::new(b"\x0812345678");
    assert_eq!(reader.read_string(1, 4).unwrap(), "1234");
    reader.finish(0x01).unwrap();
}

#[test]
fn read_string_trims_trailing_nul() {
    let mut reader = TlvReader::new(b"\x05ab\0\0\0");
    assert_eq!(reader.read_string(1, 0).unwrap(), "ab");
}

#[test]
fn truncated_reads_fail() {
    let mut reader = TlvReader::new(&[]);
    assert_eq!(reader.read_u8(), Err(Error::Truncated));

    let mut reader = TlvReader::new(&[0x01]);
    assert_eq!(reader.read_u16(Endian::Little), Err(Error::Truncated));
    assert_eq!(reader.read_u32(Endian::Little), Err(Error::Truncated));
    assert_eq!(reader.read_bytes(2), Err(Error::Truncated));
}

#[test]
fn finish_rejects_trailing_bytes() {
    let mut reader = TlvReader::new(&[0x01, 0x02]);
    reader.read_u8().unwrap();
    assert_eq!(reader.finish(0x07), Err(Error::TrailingBytes { id: 0x07 }));
}

#[test]
fn invalid_utf8_string_fails() {
    let mut reader = TlvReader::new(b"\x02\xff\xfe");
    assert_eq!(reader.read_string(1, 0), Err(Error::InvalidString));
}

#[test]
fn oversized_uint_width_fails() {
    let mut writer = TlvWriter::new();
    assert_eq!(
        writer.write_uint(0, 9, Endian::Little),
        Err(Error::InvalidLength)
    );

    let mut reader = TlvReader::new(&[0u8; 9]);
    assert_eq!(
        reader.read_uint(9, Endian::Little),
        Err(Error::InvalidLength)
    );
}
