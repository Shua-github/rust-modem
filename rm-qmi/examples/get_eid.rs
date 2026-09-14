//! Read the eID of the card in slot one, over USB or over QRTR.
//!
//! ```bash
//! cargo run -p rm-qmi --example get_eid
//! cargo run -p rm-qmi --features qrtr --example get_eid -- qrtr
//! cargo run -p rm-qmi --features qrtr --example get_eid -- qrtr 2
//! ```

use rm_client_core::LocalUim;
#[cfg(not(target_os = "android"))]
use rm_usb_core::{UsbBus, UsbBusExt};
#[cfg(not(target_os = "android"))]
use rm_usb_nusb::NusbBus;

type Failure = Box<dyn std::error::Error>;

#[cfg(not(target_os = "android"))]
type Bus = NusbBus;
#[cfg(not(target_os = "android"))]
type Device = <NusbBus as UsbBus>::Device;
#[cfg(not(target_os = "android"))]
type Client = rm_qmi::UsbQmiClient<Device>;

/// UIM slot, numbered from one, when none is given on the command line.
const SLOT: u8 = 1;

/// ISD-R, the eUICC application that carries the ES10 commands.
const ISD_R_AID: [u8; 16] = [
    0xA0, 0x00, 0x00, 0x05, 0x59, 0x10, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0x89, 0x00, 0x00, 0x01, 0x00,
];

/// ES10c GetEuiccData asking for the tag list `[5A]`, the EID.
///
/// Case 4 APDU: four header bytes, `Lc = 0x06`, the data `BF3E 03 5C 01 5A`
/// (`BF3E` GetEuiccDataRequest carrying the tag list `5C 01 5A`), then `Le`.
const GET_EID: [u8; 12] = [
    0x80, 0xE2, 0x91, 0x00, 0x06, 0xBF, 0x3E, 0x03, 0x5C, 0x01, 0x5A, 0x00,
];

/// GET RESPONSE, with `Le` the byte count the card reported in `61 xx`.
const GET_RESPONSE: [u8; 4] = [0x80, 0xC0, 0x00, 0x00];

/// Stop pulling after this many rounds; an ES10 answer is never this big.
const MAX_ROUNDS: usize = 32;

fn main() -> Result<(), Failure> {
    let transport = std::env::args().nth(1).unwrap_or_default();
    let slot = match std::env::args().nth(2) {
        Some(slot) => slot.parse()?,
        None => SLOT,
    };

    futures::executor::block_on(async {
        match transport.as_str() {
            // Android has no USB enumeration to hand out: a device there comes
            // from the descriptor the app opened, so only QRTR is reachable.
            #[cfg(not(target_os = "android"))]
            "" | "usb" => {
                let mut bus = Bus::new();
                let mut client = bus.request::<Client>().await?;

                report(read_eid(&mut client, slot).await?)
            }
            #[cfg(feature = "qrtr")]
            "qrtr" => {
                #[cfg(any(target_os = "linux", target_os = "android"))]
                {
                    let transport = rm_qmi::QrtrTransport::open_socket()?;
                    let mut client = rm_qmi::QmiClient::new(transport);

                    report(read_eid(&mut client, slot).await?)
                }

                #[cfg(not(any(target_os = "linux", target_os = "android")))]
                Err("QRTR is only reachable on Linux and Android".into())
            }
            other => Err(format!("unknown transport {other:?}, expected `usb` or `qrtr`").into()),
        }
    })
}

/// Print what the card answered, and the eID out of it.
fn report(answer: Vec<u8>) -> Result<(), Failure> {
    println!("response  {}", hex::encode_upper(&answer));
    println!("tlvs");
    print_tlvs(&answer, 1)?;

    match find_tlv(&answer, 0x5A) {
        Some(eid) => println!("eid       {}", hex::encode_upper(&eid)),
        None => println!("eid       not in the answer"),
    }

    Ok(())
}

/// Open the eUICC's ES10 channel, ask for the eID, and close it again.
async fn read_eid<C>(client: &mut C, slot: u8) -> Result<Vec<u8>, Failure>
where
    C: LocalUim,
    C::Error: 'static,
{
    let channel = client.open_channel(slot, &ISD_R_AID).await?;

    let answer = read_get_euicc_data(client, slot, channel).await;
    let closed = client.close_channel(slot, channel).await;

    let answer = answer?;
    closed?;

    Ok(answer)
}

async fn read_get_euicc_data<C>(client: &mut C, slot: u8, channel: u8) -> Result<Vec<u8>, Failure>
where
    C: LocalUim,
    C::Error: 'static,
{
    let mut answer = client.transmit(slot, channel, &GET_EID).await?;

    for _ in 0..MAX_ROUNDS {
        let (body, sw1, sw2) = split_status_word(&answer)?;

        match (sw1, sw2) {
            (0x61, available) => {
                let mut get_response = GET_RESPONSE;
                get_response[3] = available;

                let chunk = client.transmit(slot, channel, &get_response).await?;

                answer = body.to_vec();
                answer.extend_from_slice(&chunk);
            }
            (0x90, 0x00) => return Ok(body.to_vec()),
            _ => return Err(format!("card answered SW {sw1:02X}{sw2:02X}").into()),
        }
    }

    Err(format!("still fetching after {MAX_ROUNDS} GET RESPONSE rounds").into())
}

fn split_status_word(response: &[u8]) -> Result<(&[u8], u8, u8), Failure> {
    if response.len() < 2 {
        return Err(format!(
            "APDU response of {} bytes carries no status word",
            response.len()
        )
        .into());
    }

    let (body, status) = response.split_at(response.len() - 2);
    Ok((body, status[0], status[1]))
}

#[derive(Debug)]
struct Tlv<'a> {
    tag: u32,
    constructed: bool,
    value: &'a [u8],
}

fn parse_tlvs(data: &[u8]) -> Result<Vec<Tlv<'_>>, Failure> {
    let mut elements = Vec::new();
    let mut rest = data;

    while !rest.is_empty() {
        let (element, tail) = parse_tlv(rest)?;
        elements.push(element);
        rest = tail;
    }

    Ok(elements)
}

fn parse_tlv(data: &[u8]) -> Result<(Tlv<'_>, &[u8]), Failure> {
    let (&first, mut rest) = data.split_first().ok_or("tag is missing")?;

    let mut tag = u32::from(first);

    if first & 0x1F == 0x1F {
        loop {
            let (&byte, tail) = rest.split_first().ok_or("tag runs past the end")?;
            rest = tail;
            tag = (tag << 8) | u32::from(byte);

            if byte & 0x80 == 0 {
                break;
            }

            if tag > 0xFF_FF {
                return Err("tag is longer than three bytes".into());
            }
        }
    }

    let (&length, tail) = rest.split_first().ok_or("length is missing")?;
    rest = tail;

    let length = match length {
        0xFF => return Err("indefinite lengths do not appear in ES10 answers".into()),
        byte if byte & 0x80 == 0 => usize::from(byte),
        byte => {
            let width = usize::from(byte & 0x7F);
            if width > size_of::<usize>() {
                return Err("length is wider than a usize".into());
            }

            let (bytes, tail) = rest
                .split_at_checked(width)
                .ok_or("length runs past the end")?;
            rest = tail;

            bytes
                .iter()
                .fold(0usize, |length, byte| (length << 8) | usize::from(*byte))
        }
    };

    let (value, tail) = rest
        .split_at_checked(length)
        .ok_or("value runs past the end")?;

    Ok((
        Tlv {
            tag,
            constructed: first & 0x20 != 0,
            value,
        },
        tail,
    ))
}

fn find_tlv(data: &[u8], tag: u32) -> Option<Vec<u8>> {
    for element in parse_tlvs(data).ok()? {
        if element.tag == tag {
            return Some(element.value.to_vec());
        }

        if element.constructed
            && let Some(found) = find_tlv(element.value, tag)
        {
            return Some(found);
        }
    }

    None
}

fn print_tlvs(data: &[u8], depth: usize) -> Result<(), Failure> {
    for element in parse_tlvs(data)? {
        let indent = "  ".repeat(depth);
        let (tag, length, value) = (element.tag, element.value.len(), element.value);
        let width = if tag > 0xFF { 4 } else { 2 };

        match tag {
            0x5A => println!(
                "{indent}{tag:0width$X}  len={length:<3}  EID = {}",
                hex::encode_upper(value)
            ),
            _ if element.constructed => {
                println!("{indent}{tag:0width$X}  len={length:<3}  {}", tag_name(tag));
                print_tlvs(value, depth + 1)?;
            }
            _ => println!(
                "{indent}{tag:0width$X}  len={length:<3}  {}",
                hex::encode_upper(value)
            ),
        }
    }

    Ok(())
}

fn tag_name(tag: u32) -> &'static str {
    match tag {
        0x5C => "tag list",
        0xBF3E => "ES10c GetEuiccDataResponse",
        _ => "constructed",
    }
}
