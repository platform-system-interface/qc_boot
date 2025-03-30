use std::{fs::File, io::BufReader};

use clap::{Parser, Subcommand};

mod errors;
mod hwids;
mod protocol;

// TODO
const SRAM_RUN_BASE: &str = "0x00000000";

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
    /// End the transfer?
    #[clap(verbatim_doc_comment)]
    End,
    /// Reset the platform (?)
    #[clap(verbatim_doc_comment)]
    Reset,
    /// Dump memory to file
    #[clap(verbatim_doc_comment)]
    Read {
        #[clap(long, short, value_parser=clap_num::maybe_hex::<u32>, default_value = SRAM_RUN_BASE)]
        address: u32,
        file_name: String,
    },
    /// Parse MBN binary file
    #[clap(verbatim_doc_comment)]
    Parse { file_name: String },
    /// Run binary code from file
    #[clap(verbatim_doc_comment)]
    Run {
        #[clap(long, short, value_parser=clap_num::maybe_hex::<u32>, default_value = SRAM_RUN_BASE)]
        address: u32,
        file_name: String,
    },
}

/// Qualcomm mask ROM loader tool
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Command to run
    #[command(subcommand)]
    cmd: Command,
}

fn main() {
    // Default to log level "info". Otherwise, you get no "regular" logs.
    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::Builder::from_env(env).init();

    let Cli { cmd } = Cli::parse();

    match cmd {
        Command::Info => {
            let (i, e_in_addr, e_out_addr) = protocol::connect();
            let version = protocol::hello(&i, e_in_addr);

            protocol::switch_mode(&i, version, e_in_addr, e_out_addr, protocol::Mode::Command);
            protocol::info(&i, version, e_in_addr, e_out_addr)
        }
        Command::End => {
            let (i, e_in_addr, e_out_addr) = protocol::connect();
            let version = protocol::hello(&i, e_in_addr);

            protocol::end(&i, version, e_in_addr, e_out_addr);
        }
        Command::Reset => {
            let (i, e_in_addr, e_out_addr) = protocol::connect();
            let version = protocol::hello(&i, e_in_addr);

            protocol::reset(&i, e_in_addr, e_out_addr);
        }
        Command::Read { address, file_name } => {
            let (i, e_in_addr, e_out_addr) = protocol::connect();
            let version = protocol::hello(&i, e_in_addr);

            protocol::switch_mode(
                &i,
                version,
                e_in_addr,
                e_out_addr,
                protocol::Mode::MemoryDebug,
            );
            protocol::read_mem(&i, version, e_in_addr, e_out_addr, address);
        }
        Command::Parse { file_name } => {
            match mbn::from_elf(file_name.clone()) {
                Ok(seg) => {
                    let h = seg.mbn_header;
                    println!("{h:#02x?}");
                    return;
                }
                Err(e) => println!("Cannot parse as ELF: {e:#02x?}"),
            };
            let data = std::fs::read(file_name).unwrap();
            match mbn::HashTableSegment::parse(&data) {
                Ok(seg) => {
                    let h = seg.mbn_header;
                    println!("{h:?}");
                }
                Err(e) => println!("Cannot parse raw hash table segment: {e:#02x?}"),
            };
        }
        // TODO
        _ => {}
    }
}
