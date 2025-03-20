use std::io::{self, ErrorKind::TimedOut, Read, Result};
use std::str::from_utf8;
use std::thread;
use std::time::{Duration, Instant};
use std::{fs::File, io::BufReader};

use async_io::{Timer, block_on};
use clap::{Parser, Subcommand};
use futures_lite::FutureExt;
use log::{debug, error, info};
use nusb::{
    Device, Interface, Speed,
    transfer::{ControlIn, ControlOut, ControlType, Direction, Recipient, RequestBuffer},
};

use zerocopy::{FromBytes, IntoBytes};
use zerocopy_derive::{FromBytes, Immutable, IntoBytes};

mod hwids;

const QUALCOMM_VID: u16 = 0x05c6;
const XX_PID: u16 = 0x9008;

const CLAIM_INTERFACE_TIMEOUT: Duration = Duration::from_secs(1);
const CLAIM_INTERFACE_PERIOD: Duration = Duration::from_micros(200);

// TODO
const SRAM_RUN_BASE: &str = "0x00000000";

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

// protocol thingies
const COMMAND_HELLO: u32 = 2;
const COMMAND_END_OF_TRANSFER: u32 = 4;
const COMMAND_READY: u32 = 0xb;
const COMMAND_EXECUTE_REQUEST: u32 = 0xd;
const COMMAND_EXECUTE_RESPONSE: u32 = 0xe;
const COMMAND_EXECUTE_DATA: u32 = 0xf;

// protocol modes
const MODE_COMMAND: u32 = 3;

// actual commands
const EXEC_SERIAL_NUM_READ: u32 = 0x01;
const EXEC_MSM_HW_ID_READ: u32 = 0x02;

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

fn hello(i: &Interface, e_in_addr: u8) {
    let b = &usb_read(i, e_in_addr)[..32];
    debug!("Device says: {b:02x?}");
    let (req, _) = HelloRequest::read_from_prefix(b).unwrap();
    debug!("Request: {req:#02x?}");
}

fn info(i: &Interface, e_in_addr: u8, e_out_addr: u8) {
    let res = HelloResponse {
        header: PacketHeader {
            command: COMMAND_HELLO,
            length: 0x30,
        },
        version: 2,
        compatible: 1, // aka version_min
        status: 0,     // aka max_cmd_len
        mode: MODE_COMMAND,
    };

    debug!("send {res:#02x?}");

    let mut r = res.as_bytes().to_vec();
    let wtf = [1u32, 2, 3, 4, 5, 6].as_bytes().to_vec();

    r.append(&mut wtf.to_vec());

    usb_send(i, e_out_addr, r);
    let b = &usb_read(i, e_in_addr)[..32];
    debug!("Device says: {b:02x?}");

    let cmd = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);

    match cmd {
        COMMAND_READY => {
            debug!("command ready");
        }
        COMMAND_END_OF_TRANSFER => {
            let (p, _) = EndOfTransfer::read_from_prefix(b).unwrap();
            panic!("{p:#02x?}");
        }
        _ => panic!("..."),
    }

    let r = [COMMAND_EXECUTE_REQUEST, 0xc, EXEC_MSM_HW_ID_READ].as_bytes();
    usb_send(i, e_out_addr, r.to_vec());

    let b = &usb_read(i, e_in_addr)[..32];
    debug!("Device says: {b:02x?}");

    let cmd = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);

    match cmd {
        COMMAND_EXECUTE_RESPONSE => {
            debug!("execute response");
        }
        _ => panic!("..."),
    }

    let r = [COMMAND_EXECUTE_DATA, 0xc, EXEC_MSM_HW_ID_READ].as_bytes();
    usb_send(i, e_out_addr, r.to_vec());

    let b = &usb_read(i, e_in_addr);
    let id = [b[4], b[5], b[6], b[7]];
    let id = u32::from_le_bytes(id);
    let name = hwids::hwid_to_name(id);
    println!("MSM hardware ID: {id:08x} ({name})");

    let r = [COMMAND_EXECUTE_REQUEST, 0xc, EXEC_SERIAL_NUM_READ].as_bytes();
    usb_send(i, e_out_addr, r.to_vec());

    let b = &usb_read(i, e_in_addr)[..32];
    debug!("Device says: {b:02x?}");

    let cmd = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);

    match cmd {
        COMMAND_EXECUTE_RESPONSE => {
            debug!("execute response");
        }
        _ => panic!("..."),
    }

    let r = [COMMAND_EXECUTE_DATA, 0xc, EXEC_SERIAL_NUM_READ].as_bytes();
    usb_send(i, e_out_addr, r.to_vec());

    let b = &usb_read(i, e_in_addr)[..8];
    let mut id = b.to_vec();
    id.reverse();
    println!("Serial number: {id:02x?}");
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print CPU info
    #[clap(verbatim_doc_comment)]
    Info,
    /// Load binary from file to memory
    #[clap(verbatim_doc_comment)]
    Load {
        #[clap(long, short, value_parser=clap_num::maybe_hex::<u32>, default_value = SRAM_RUN_BASE)]
        address: u32,
        file_name: String,
    },
    /// Run binary code from file
    #[clap(verbatim_doc_comment)]
    Run {
        #[clap(long, short, value_parser=clap_num::maybe_hex::<u32>, default_value = SRAM_RUN_BASE)]
        address: u32,
        file_name: String,
    },
}

/// Kendryte mask ROM loader tool
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Command to run
    #[command(subcommand)]
    cmd: Command,
}

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

fn main() {
    // Default to log level "info". Otherwise, you get no "regular" logs.
    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::Builder::from_env(env).init();

    let cmd = Cli::parse().cmd;

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

    hello(&i, e_in_addr);

    match cmd {
        Command::Info => info(&i, e_in_addr, e_out_addr),
        // TODO
        _ => {}
    }
}
