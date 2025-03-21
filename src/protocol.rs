use std::io::{self, ErrorKind::TimedOut, Read, Result};
use std::thread;
use std::time::{Duration, Instant};

use async_io::{Timer, block_on};
use futures_lite::FutureExt;
use log::{debug, error, info};
use nusb::{
    Device, Interface, Speed,
    transfer::{Direction, RequestBuffer},
};
use zerocopy::{FromBytes, IntoBytes};
use zerocopy_derive::{FromBytes, Immutable, IntoBytes};

use crate::hwids;

const QUALCOMM_VID: u16 = 0x05c6;
const XX_PID: u16 = 0x9008;

const CLAIM_INTERFACE_TIMEOUT: Duration = Duration::from_secs(1);
const CLAIM_INTERFACE_PERIOD: Duration = Duration::from_micros(200);

/* from https://git.codelinaro.org/linaro/qcomlt/qdl.git
    if (ifc->bInterfaceClass != 0xff)
        continue;

    if (ifc->bInterfaceSubClass != 0xff)
        continue;

    /* bInterfaceProtocol of 0xff, 0x10 and 0x11 has been seen */
    if (ifc->bInterfaceProtocol != 0xff &&
        ifc->bInterfaceProtocol != 16 &&
        ifc->bInterfaceProtocol != 17)
        continue;
*/

fn claim_interface(d: &Device, ii: u8) -> std::result::Result<Interface, String> {
    let now = Instant::now();
    while Instant::now() <= now + CLAIM_INTERFACE_TIMEOUT {
        match d.claim_interface(ii) {
            Ok(i) => {
                return Ok(i);
            }
            Err(_) => {
                thread::sleep(CLAIM_INTERFACE_PERIOD);
            }
        }
    }
    Err("failure claiming USB interface".into())
}

pub fn connect() -> (Interface, u8, u8) {
    let di = nusb::list_devices()
        .unwrap()
        .find(|d| d.vendor_id() == QUALCOMM_VID && d.product_id() == XX_PID)
        .expect("Device not found, is it connected and in the right mode?");
    let ms = di.manufacturer_string().unwrap_or("[no manufacturer]");
    let ps = di.product_string().unwrap();
    info!("Found {ms} {ps}");

    // Just use the first interface
    let ii = di.interfaces().next().unwrap().interface_number();
    let d = di.open().unwrap();
    let i = claim_interface(&d, ii).unwrap();

    let speed = di.speed().unwrap();
    let packet_size = match speed {
        Speed::Full | Speed::Low => 64,
        Speed::High => 512,
        Speed::Super | Speed::SuperPlus => 1024,
        _ => panic!("Unknown USB device speed {speed:?}"),
    };
    debug!("speed {speed:?} - max packet size: {packet_size}");

    // TODO: Nice error messages when either is not found
    // We may also hardcode the endpoint to 0x01.
    let c = d.configurations().next().unwrap();
    let s = c.interface_alt_settings().next().unwrap();

    let mut es = s.endpoints();
    let e_out = es.find(|e| e.direction() == Direction::Out).unwrap();
    let e_out_addr = e_out.address();

    let mut es = s.endpoints();
    let e_in = es.find(|e| e.direction() == Direction::In).unwrap();
    let e_in_addr = e_in.address();

    for e in es {
        debug!("{e:?}");
    }

    (i, e_in_addr, e_out_addr)
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct PacketHeader {
    command: u32,
    length: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct HelloRequest {
    header: PacketHeader,
    version: u32,
    compatible: u32,
    max_len: u32,
    mode: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct HelloResponse {
    header: PacketHeader,
    version: u32,
    compatible: u32,
    status: u32,
    mode: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct ReadRequest32 {
    header: PacketHeader,
    image: u32,
    offset: u32,
    length: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct ReadRequest64 {
    header: PacketHeader,
    image: u64,
    offset: u64,
    length: u64,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct EndOfTransfer {
    header: PacketHeader,
    image: u32,
    status: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct DoneResponse {
    header: PacketHeader,
    status: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct HardwareId {
    model: u16,
    oem: u16,
    id: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct SerialNo {
    serial: [u8; 8],
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
#[repr(C, packed)]
struct OemPkHash {
    hash1: [u8; 32],
    hash2: [u8; 32],
    hash3: [u8; 32],
}

// protocol thingies
const COMMAND_HELLO_REQUEST: u32 = 1;
const COMMAND_HELLO_RESPONSE: u32 = 2;
const COMMAND_END_OF_TRANSFER: u32 = 4;
const COMMAND_READY: u32 = 0xb;
const COMMAND_EXECUTE_REQUEST: u32 = 0xd;
const COMMAND_EXECUTE_RESPONSE: u32 = 0xe;
const COMMAND_EXECUTE_DATA: u32 = 0xf;

// protocol modes
const MODE_COMMAND: u32 = 3;

// actual commands
const EXEC_GET_SERIAL_NUM: u32 = 0x01;
const EXEC_GET_HARDWARE_ID: u32 = 0x02;
const EXEC_GET_OEM_PK_HASH: u32 = 0x03;
const EXEC_GET_SBL_VERSION: u32 = 0x07;
const EXEC_GET_COMMAND_ID_LIST: u32 = 0x08;
const EXEC_GET_TRAINING_DATA: u32 = 0x09;

const TRANSFER_SIZE: usize = 4096;

fn usb_read(i: &Interface, addr: u8) -> [u8; TRANSFER_SIZE] {
    let mut buf = [0_u8; TRANSFER_SIZE];

    let _: Result<usize> = {
        let timeout = Duration::from_secs(5);
        let fut = async {
            let b = RequestBuffer::new(TRANSFER_SIZE);
            let comp = i.bulk_in(addr, b).await;
            comp.status.map_err(io::Error::other)?;

            let n = comp.data.len();
            buf[..n].copy_from_slice(&comp.data);
            Ok(n)
        };

        block_on(fut.or(async {
            Timer::after(timeout).await;
            Err(TimedOut.into())
        }))
    };

    let b = &buf[..128];
    debug!("Device says: {b:02x?}");

    buf
}

fn usb_send(i: &Interface, addr: u8, data: Vec<u8>) {
    let _: Result<usize> = {
        let timeout = Duration::from_secs(5);
        let fut = async {
            let comp = i.bulk_out(addr, data).await;
            comp.status.map_err(io::Error::other)?;
            let n = comp.data.actual_length();
            Ok(n)
        };

        block_on(fut.or(async {
            Timer::after(timeout).await;
            Err(TimedOut.into())
        }))
    };
}

pub fn hello(i: &Interface, e_in_addr: u8) {
    let b = &usb_read(i, e_in_addr);
    let (req, _) = HelloRequest::read_from_prefix(b).unwrap();
    debug!("Request: {req:#02x?}");
    let cmd = req.header.command;
    assert_eq!(cmd, COMMAND_HELLO_REQUEST);
}

fn switch_mode_to_command(i: &Interface, e_in_addr: u8, e_out_addr: u8) {
    // As unusual as it is, we get a _request_ first, so we _send a response_.
    // See hello() in which we take the request.
    let res = HelloResponse {
        header: PacketHeader {
            command: COMMAND_HELLO_RESPONSE,
            length: 0x30,
        },
        version: 2,
        compatible: 1, // aka version_min
        status: 0,     // aka max_cmd_len
        mode: MODE_COMMAND,
    };
    debug!("send {res:#02x?}");
    let mut r = res.as_bytes().to_vec();
    // We got this from https://github.com/bkerler/edl.
    // see edlclient/Library/sahara.py cmd_hello()
    let ottffs = [1u32, 2, 3, 4, 5, 6].as_bytes().to_vec();
    r.append(&mut ottffs.to_vec());
    usb_send(i, e_out_addr, r);

    let b = &usb_read(i, e_in_addr);
    let (header, _) = PacketHeader::read_from_prefix(b).unwrap();
    let cmd = header.command;
    assert_eq!(cmd, COMMAND_READY);
}

// NOTE: This is a two-step thing. Read the data response afterwards,
fn exec(
    i: &Interface,
    e_in_addr: u8,
    e_out_addr: u8,
    exec_cmd: u32,
) -> std::result::Result<(), String> {
    // TODO: struct
    let r = [COMMAND_EXECUTE_REQUEST, 0xc, exec_cmd];
    usb_send(i, e_out_addr, r.as_bytes().to_vec());

    let b = &usb_read(i, e_in_addr);
    let (header, _) = PacketHeader::read_from_prefix(b).unwrap();
    let cmd = header.command;
    // NOTE: this may get a SAHARA_END_TRANSFER; may mean command not found
    if cmd != COMMAND_EXECUTE_RESPONSE {
        return Err(format!("xx maybe cmd invalid {cmd:02x}"));
    }

    let r = [COMMAND_EXECUTE_DATA, 0xc, exec_cmd];
    usb_send(i, e_out_addr, r.as_bytes().to_vec());
    Ok(())
}

pub fn info(i: &Interface, e_in_addr: u8, e_out_addr: u8) {
    switch_mode_to_command(i, e_in_addr, e_out_addr);
    exec(i, e_in_addr, e_out_addr, EXEC_GET_HARDWARE_ID).unwrap();
    let b = &usb_read(i, e_in_addr);
    let (d, _) = HardwareId::read_from_prefix(b).unwrap();
    let HardwareId { model, oem, id } = d;
    let name = hwids::hwid_to_name(id);
    println!("Hardware ID: {id:08x} ({name})");
    // TODO: map OEM + model to string names
    println!("OEM: {model:04x}");
    println!("Model: {oem:04x}");

    exec(i, e_in_addr, e_out_addr, EXEC_GET_SERIAL_NUM).unwrap();
    let b = &usb_read(i, e_in_addr);
    let (d, _) = SerialNo::read_from_prefix(b).unwrap();
    // TODO: Which bytes do we really need?
    let serial = d.serial;
    println!("Serial number: {serial:02x?}");

    exec(i, e_in_addr, e_out_addr, EXEC_GET_OEM_PK_HASH).unwrap();
    let b = &usb_read(i, e_in_addr);
    // There is a condition in https://github.com/bkerler/edl that searches for
    // a second occurrence of the first 4 bytes again in the other bytes, then
    // takes [4+p..], where p is the position where it is found again. Wtf?
    // AFAICT, there is just 3x the same hash.
    let (d, _) = OemPkHash::read_from_prefix(b).unwrap();
    let OemPkHash {
        hash1,
        hash2,
        hash3,
    } = d;
    println!("OEM PK hashes:");
    println!("  {hash1:02x?}");
    println!("  {hash2:02x?}");
    println!("  {hash3:02x?}");
}
