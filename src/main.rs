use std::{fs::File, io::BufReader};

use clap::{Parser, Subcommand};

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

    let cmd = Cli::parse().cmd;

    let (i, e_in_addr, e_out_addr) = protocol::connect();

    protocol::hello(&i, e_in_addr);

    match cmd {
        Command::Info => protocol::info(&i, e_in_addr, e_out_addr),
        // TODO
        _ => {}
    }
}
