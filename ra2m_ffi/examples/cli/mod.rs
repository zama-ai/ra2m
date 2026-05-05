//!
//! Define commoon client CLI arguments

use clap::{Parser, Subcommand};
use clap_num::maybe_hex;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    #[clap(about = "Memory Read")]
    Read {
        /// Start addr
        #[arg(short, long, value_parser=maybe_hex::<u64>, default_value_t = 0)]
        addr: u64,
        /// read size in bytes
        #[arg(short, long, value_parser=maybe_hex::<u64>, default_value_t = 64)]
        size_b: u64,
    },

    #[clap(about = "Memory Write")]
    Write {
        /// Start addr
        #[arg(short, long, value_parser=maybe_hex::<u64>, default_value_t = 0)]
        addr: u64,
        /// write size in bytes
        #[arg(short, long, value_parser=maybe_hex::<u64>, default_value_t = 64)]
        size_b: u64,
        /// Optional pattern
        /// Force a pattern instead of random value
        #[arg(short, long, value_parser=maybe_hex::<u64>)]
        pattern: Option<u64>,
    },
}

#[derive(Clone, Debug, Parser)]
pub struct CliArgs {
    // bridge configuration -----------------------------------------------------
    #[arg(short, long)]
    pub name: String,

    #[command(subcommand)]
    pub cmd: Command,
}
