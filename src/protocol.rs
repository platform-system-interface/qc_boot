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
    message_type: u32,
    length: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct HelloRequest {
    header: PacketHeader,
    version: u32,
    compatible: u32,
    max_len: u32, // max msg/"cmd" length
    mode: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct HelloResponse {
    header: PacketHeader,
    version: u32,
    compatible: u32, // aka version_min
    status: u32,     // aka max_cmd_len
    mode: u32,
    rest: [u32; 6],
}

const HELLO_RESPONSE_SIZE: u32 = std::mem::size_of::<HelloResponse>() as u32;

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct ResetRequest {
    header: PacketHeader,
}

const RESET_REQUEST_SIZE: u32 = std::mem::size_of::<ResetRequest>() as u32;

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct DoneRequest {
    header: PacketHeader,
}

const DONE_REQUEST_SIZE: u32 = std::mem::size_of::<DoneRequest>() as u32;

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct ReadRequest32 {
    header: PacketHeader,
    image: u32,
    offset: u32,
    length: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct ReadRequest64 {
    header: PacketHeader,
    image: u64,
    offset: u64,
    length: u64,
}

/// Response to all sorts of requests, may indicate an error.
#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct EndOfTransfer {
    header: PacketHeader,
    image: u32,
    status: u32,
}

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct DoneResponse {
    header: PacketHeader,
    status: u32,
}

/// This is both for request and data.
#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct Exec {
    header: PacketHeader,
    command: u32,
}

const EXEC_SIZE: u32 = std::mem::size_of::<Exec>() as u32;

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct MemoryRead32 {
    header: PacketHeader,
    address: u32,
    size: u32,
}

const MEMORY_READ_SIZE: u32 = core::mem::size_of::<MemoryRead32>() as u32;

/* ----- command exec response data ----- */

/// Response data to hardware ID command.
#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct HardwareId {
    model: u16,
    oem: u16,
    id: u32,
}

/// Response data to serial number command.
#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct SerialNo {
    serial: [u8; 8],
}

/// Response data to OEM PK hash command.
#[derive(Clone, Debug, Copy, FromBytes, IntoBytes, Immutable)]
#[repr(C, packed)]
struct OemPkHash {
    hash1: [u8; 32],
    hash2: [u8; 32],
    hash3: [u8; 32],
}

// Sahara message types
const SAHARA_HELLO_REQUEST: u32 = 0x1;
const SAHARA_HELLO_RESPONSE: u32 = 0x2;
const SAHARA_READ_DATA: u32 = 0x3;
const SAHARA_END_OF_TRANSFER: u32 = 0x4;
const SAHARA_DONE_REQUEST: u32 = 0x5;
const SAHARA_DONE_RESPONSE: u32 = 0x6;
const SAHARA_RESET_REQUEST: u32 = 0x7;
const SAHARA_RESET_RESPONSE: u32 = 0x8;
const SAHARA_MEMORY_DEBUG: u32 = 0x9;
const SAHARA_MEMORY_READ: u32 = 0xa;
const SAHARA_READY: u32 = 0xb;
// TODO: use this
const SAHARA_SWITCH_MODE: u32 = 0xc;
const SAHARA_EXECUTE_REQUEST: u32 = 0xd;
const SAHARA_EXECUTE_RESPONSE: u32 = 0xe;
const SAHARA_EXECUTE_DATA: u32 = 0xf;
const SAHARA_64BIT_MEMORY_DEBUG: u32 = 0x10;
const SAHARA_64BIT_MEMORY_READ: u32 = 0x11;
const SAHARA_64BIT_MEMORY_READ_DATA: u32 = 0x12;
const SAHARA_RESET_STATE_MACHINE_ID: u32 = 0x13;

// protocol modes
#[repr(u32)]
pub enum Mode {
    ImageTxPending = 0,
    ImageTxComplete = 1,
    MemoryDebug = 2,
    Command = 3,
}

#[repr(u32)]
#[derive(Clone, Debug, Copy, IntoBytes, Immutable)]
enum Command {
    None = 0,
    GetSerialNum = 0x01,
    GetHardwareId = 0x02,
    GetOemPkHash = 0x03,
    GetSblVersion = 0x07,
    GetCommandIdList = 0x08,
    GetTrainingData = 0x09,
}

// Taken from Linaro's code
// const TRANSFER_SIZE: usize = 0x1000;
// Should suffice; we get this as max_len in chips we tried.
const TRANSFER_SIZE: usize = 0x400;

fn usb_read_n(i: &Interface, addr: u8, size: usize) -> Vec<u8> {
    let mut buf = vec![0_u8; size];

    let _: Result<usize> = {
        let timeout = Duration::from_secs(5);
        let fut = async {
            let b = RequestBuffer::new(size);
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

    let l = if buf.len() < 128 { buf.len() } else { 128 };
    let b = &buf[..l];
    debug!("Device says: {b:02x?}");

    buf
}

fn usb_read(i: &Interface, addr: u8) -> Vec<u8> {
    usb_read_n(i, addr, TRANSFER_SIZE)
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

// TODO: return Mode
pub fn hello(i: &Interface, e_in_addr: u8) -> u32 {
    let b = &usb_read(i, e_in_addr);
    let (req, _) = HelloRequest::read_from_prefix(b).unwrap();
    info!("Hello request: {req:#02x?}");
    let mt = req.header.message_type;
    assert_eq!(mt, SAHARA_HELLO_REQUEST);
    req.mode
}

pub fn switch_mode(i: &Interface, version: u32, e_in_addr: u8, e_out_addr: u8, mode: Mode) {
    // As unusual as it is, we get a _request_ first, so we _send a response_.
    // See hello() in which we take the request.
    let res = HelloResponse {
        header: PacketHeader {
            message_type: SAHARA_HELLO_RESPONSE,
            length: HELLO_RESPONSE_SIZE,
        },
        version,
        compatible: 1,
        status: 0,
        mode: mode as u32,
        // The protocol expects either 0x14 or 0x30 bytes. Fill the rest.
        rest: [0, 0, 0, 0, 0, 0],
    };
    debug!("send {res:#02x?}");
    let r = res.as_bytes().to_vec();
    usb_send(i, e_out_addr, r);

    let b = &usb_read(i, e_in_addr);
    let (header, _) = PacketHeader::read_from_prefix(b).unwrap();
    let mt = header.message_type;
    if mt == SAHARA_END_OF_TRANSFER {
        let (eot, _) = EndOfTransfer::read_from_prefix(b).unwrap();
        let status = eot.status;
        let msg = crate::errors::error_code_to_str(status);
        error!("Mode switch failed with status {status:02x}: {msg}");
    }
    if mt != SAHARA_READY {
        println!("Mode switch failed, got message: {mt:02x}");
        // panic!();
    }
}

// NOTE: This is a two-step thing. Read the data response afterwards,
fn exec(
    i: &Interface,
    e_in_addr: u8,
    e_out_addr: u8,
    command: Command,
) -> std::result::Result<(), String> {
    let cmd = command as u32;
    let packet = Exec {
        header: PacketHeader {
            message_type: SAHARA_EXECUTE_REQUEST,
            length: EXEC_SIZE,
        },
        command: command as u32,
    };
    let r = packet.as_bytes().to_vec();
    usb_send(i, e_out_addr, r);

    let b = &usb_read(i, e_in_addr);
    let (header, _) = PacketHeader::read_from_prefix(b).unwrap();
    let mt = header.message_type;
    if mt == SAHARA_END_OF_TRANSFER {
        let (eot, _) = EndOfTransfer::read_from_prefix(b).unwrap();
        let status = eot.status;
        let msg = crate::errors::error_code_to_str(status);
        return Err(format!("Command failed with status {status:02x}: {msg}"));
    }
    if mt != SAHARA_EXECUTE_RESPONSE {
        return Err(format!("Unexpected message type {mt:02x}"));
    }

    let packet = Exec {
        header: PacketHeader {
            message_type: SAHARA_EXECUTE_DATA,
            length: EXEC_SIZE,
        },
        command: command as u32,
    };
    let r = packet.as_bytes().to_vec();
    usb_send(i, e_out_addr, r);
    Ok(())
}

pub fn info(i: &Interface, version: u32, e_in_addr: u8, e_out_addr: u8) {
    exec(i, e_in_addr, e_out_addr, Command::GetSerialNum).unwrap();
    let b = &usb_read(i, e_in_addr);
    let (d, _) = SerialNo::read_from_prefix(b).unwrap();
    // TODO: Which bytes do we really need?
    let serial = d.serial;
    println!("Serial number: {serial:02x?}");

    // HWID and OEM public key hash are only for v2 and older
    if version < 3 {
        exec(i, e_in_addr, e_out_addr, Command::GetHardwareId).unwrap();
        let b = &usb_read(i, e_in_addr);
        let (d, _) = HardwareId::read_from_prefix(b).unwrap();
        let HardwareId { model, oem, id } = d;
        let name = hwids::hwid_to_name(id);
        println!("Hardware ID: {id:08x} ({name})");
        // TODO: map OEM + model to string names
        println!("OEM: {model:04x}");
        println!("Model: {oem:04x}");

        exec(i, e_in_addr, e_out_addr, Command::GetOemPkHash).unwrap();
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

    if false {
        match exec(i, e_in_addr, e_out_addr, Command::GetSblVersion) {
            Ok(()) => {
                let b = &usb_read(i, e_in_addr)[..64];
                println!("SBL version {b:02x?}");
            }
            Err(e) => {
                println!("Getting SBL version failed: {e}");
            }
        }
        match exec(i, e_in_addr, e_out_addr, Command::GetCommandIdList) {
            Ok(()) => {
                let b = &usb_read(i, e_in_addr)[..64];
                println!("Command ID list {b:02x?}");
            }
            Err(e) => {
                println!("Getting command ID list failed: {e}");
            }
        }
    }
}

pub fn run(i: &Interface, e_in_addr: u8, e_out_addr: u8) {
    //
}

pub fn read_mem(i: &Interface, version: u32, e_in_addr: u8, e_out_addr: u8, address: u32) {
    switch_mode(i, version, e_in_addr, e_out_addr, Mode::MemoryDebug);

    let size = 0x10;

    let packet = MemoryRead32 {
        header: PacketHeader {
            message_type: SAHARA_MEMORY_READ,
            length: MEMORY_READ_SIZE,
        },
        address,
        size,
    };
    let r = packet.as_bytes().to_vec();
    usb_send(i, e_out_addr, r);

    let res = &usb_read_n(i, e_in_addr, size as usize);
    let (header, _) = PacketHeader::read_from_prefix(res).unwrap();
    let mt = header.message_type;
    if mt == SAHARA_END_OF_TRANSFER {
        let (eot, _) = EndOfTransfer::read_from_prefix(res).unwrap();
        let status = eot.status;
        let msg = crate::errors::error_code_to_str(status);
        panic!("Reading memory failed with status {status:02x}: {msg}");
    }

    info!("{res:02x?}");
}

pub fn reset(i: &Interface, e_in_addr: u8, e_out_addr: u8) {
    let packet = ResetRequest {
        header: PacketHeader {
            message_type: SAHARA_RESET_REQUEST,
            length: RESET_REQUEST_SIZE,
        },
    };
    let r = packet.as_bytes().to_vec();
    usb_send(i, e_out_addr, r);

    let res = &usb_read(i, e_in_addr)[..32];
    let (header, _) = PacketHeader::read_from_prefix(res).unwrap();
    let mt = header.message_type;
    if mt == SAHARA_END_OF_TRANSFER {
        let (eot, _) = EndOfTransfer::read_from_prefix(res).unwrap();
        let status = eot.status;
        let msg = crate::errors::error_code_to_str(status);
        panic!("Reset failed with status {status:02x}: {msg}");
    }

    if mt == SAHARA_RESET_RESPONSE {
        info!("Reset successful: {res:02x?}");
    } else {
        info!("Reset got unexpected response: {res:02x?}");
    }
}

pub fn end(i: &Interface, version: u32, e_in_addr: u8, e_out_addr: u8) {
    switch_mode(i, version, e_in_addr, e_out_addr, Mode::ImageTxPending);

    let packet = DoneRequest {
        header: PacketHeader {
            message_type: SAHARA_DONE_REQUEST,
            length: DONE_REQUEST_SIZE,
        },
    };
    let r = packet.as_bytes().to_vec();
    usb_send(i, e_out_addr, r);

    let res = &usb_read(i, e_in_addr)[..32];
    let (header, _) = PacketHeader::read_from_prefix(res).unwrap();
    let mt = header.message_type;
    if mt == SAHARA_END_OF_TRANSFER {
        let (eot, _) = EndOfTransfer::read_from_prefix(res).unwrap();
        let status = eot.status;
        let msg = crate::errors::error_code_to_str(status);
        panic!("Done failed with status {status:02x}: {msg}");
    }

    if mt == SAHARA_DONE_RESPONSE {
        info!("Got done response {res:02x?}");
    }
    info!("Got  {res:02x?}");
}
