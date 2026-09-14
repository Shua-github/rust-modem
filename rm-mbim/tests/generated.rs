use rm_mbim::types::basic_connect;
use rm_mbim::types::ms_uicc_low_level_access as uicc;
use rm_mbim::{CommandType, Message, Request, Response, Uuid};

/// Compile-time check that a request and its response are bound to each other.
fn assert_binding<R, S>()
where
    R: Request<Response = S>,
    S: Response<Request = R>,
{
}

#[test]
fn generated_commands_bind_their_service_and_id() {
    assert_eq!(basic_connect::device_caps::CID, 1);
    assert_eq!(basic_connect::radio_state::CID, 3);
    assert_eq!(basic_connect::SERVICE, Uuid::BASIC_CONNECT);

    type Query = basic_connect::device_caps::QueryRequest;
    assert_eq!(<Query as Request>::SERVICE, Uuid::BASIC_CONNECT);
    assert_eq!(<Query as Request>::CID, 1);
    assert_eq!(<Query as Request>::COMMAND_TYPE, CommandType::Query);

    type Set = basic_connect::radio_state::SetRequest;
    assert_eq!(<Set as Request>::SERVICE, Uuid::BASIC_CONNECT);
    assert_eq!(<Set as Request>::CID, 3);
    assert_eq!(<Set as Request>::COMMAND_TYPE, CommandType::Set);

    assert_binding::<Query, basic_connect::device_caps::QueryResponse>();
    assert_binding::<Set, basic_connect::radio_state::SetResponse>();
}

#[test]
fn generated_query_encodes_an_empty_information_buffer() {
    type Query = basic_connect::device_caps::QueryRequest;

    let message = Message::command(
        7,
        <Query as Request>::SERVICE,
        <Query as Request>::CID,
        <Query as Request>::COMMAND_TYPE,
        &Query::default().encode(),
    );
    let info = message.command_info().unwrap();

    assert_eq!(info.service, Uuid::BASIC_CONNECT);
    assert_eq!(info.cid, basic_connect::device_caps::CID);
    assert_eq!(info.command_type, Some(CommandType::Query));
    assert!(info.information_buffer.is_empty());
}

#[test]
fn generated_set_command_encodes_its_request() {
    let request = basic_connect::radio_state::SetRequest { radio_state: 1 };
    let message = Message::command(
        9,
        basic_connect::SERVICE,
        basic_connect::radio_state::CID,
        CommandType::Set,
        &request.encode(),
    );
    let info = message.command_info().unwrap();

    assert_eq!(info.information_buffer, &1u32.to_le_bytes());
    assert_eq!(Message::from_bytes(&message.to_bytes()).unwrap(), message);
}

#[test]
fn generated_requests_decode_what_they_encoded() {
    let request = uicc::open_channel::SetRequest {
        app_id: vec![0xa0, 0x00, 0x00, 0x00],
        select_p2_arg: 0,
        channel_group: 0,
    };

    assert_eq!(
        uicc::open_channel::SetRequest::decode(&request.encode()).unwrap(),
        request
    );
}

#[test]
fn generated_uicc_types_encode_and_decode_references() {
    let request = uicc::open_channel::SetRequest {
        app_id: vec![0xa0, 0x00, 0x00, 0x00],
        select_p2_arg: 0,
        channel_group: 0,
    };
    assert_eq!(
        request.to_bytes(),
        vec![
            // UICC reference byte arrays carry the length first, then the offset
            // (libmbim's `swapped_offset_length`).
            4, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa0, 0x00, 0x00, 0x00,
        ]
    );

    let information = [
        0u8, 0, 0, 0, 0x07, 0, 0, 0, 2, 0, 0, 0, 16, 0, 0, 0, 0x90, 0x00, 0, 0,
    ];
    let message = Message::command(
        1,
        uicc::SERVICE,
        uicc::cid::OPEN_CHANNEL,
        CommandType::Set,
        &information,
    );
    let response = uicc::open_channel::SetResponse::from_message(&message).unwrap();
    assert_eq!(response.status, 0);
    assert_eq!(response.channel, 7);
    assert_eq!(response.response, vec![0x90, 0x00]);
}
