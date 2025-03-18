use std::io::{self, ErrorKind::TimedOut, Read, Result};
use std::str::from_utf8;
use std::thread;
use std::time::{Duration, Instant};
use std::{fs::File, io::BufReader};

use async_io::{Timer, block_on};
use clap::{Parser, Subcommand};
use futures_lite::FutureExt;
use nusb::{
    Device, Interface, Speed,
    transfer::{ControlIn, ControlOut, ControlType, Direction, Recipient, RequestBuffer},
};

use zerocopy::{FromBytes, IntoBytes};
use zerocopy_derive::{FromBytes, IntoBytes};

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

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
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

#[derive(Clone, Debug, Copy, FromBytes, IntoBytes)]
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
struct EndOfImage {
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

const TRANSFER_SIZE: usize = 4096;

fn hello(i: &Interface, e_in_addr: u8) {
    let mut buf = [0_u8; TRANSFER_SIZE];

    let _: Result<usize> = {
        let timeout = Duration::from_secs(5);
        let fut = async {
            let b = RequestBuffer::new(TRANSFER_SIZE);
            let comp = i.bulk_in(e_in_addr, b).await;
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

    let b = &buf[..32];
    println!("Device says: {b:02x?}");

    /*
    HelloRequest {
        header: PacketHeader {
            command: 0x1,
            length: 0x30,
        },
        version: 0x2,
        compatible: 0x1,
        max_len: 0x400,
        mode: 0x0,
    },
    */

    let req = HelloRequest::read_from_prefix(b);
    println!("Request: {req:#02x?}");
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
    let cmd = Cli::parse().cmd;

    let di = nusb::list_devices()
        .unwrap()
        .find(|d| d.vendor_id() == QUALCOMM_VID && d.product_id() == XX_PID)
        .expect("Device not found, is it connected and in the right mode?");
    let ms = di.manufacturer_string().unwrap_or("[no manufacturer]");
    let ps = di.product_string().unwrap();
    println!("Found {ms} {ps}");

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
    println!("speed {speed:?} - max packet size: {packet_size}");

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
        println!("{e:?}");
    }

    hello(&i, e_in_addr);

    // TODO: commands
}
