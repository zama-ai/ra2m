use clap::Parser;
use rand::{
    rngs::StdRng,
    {Rng, SeedableRng},
};

use ra2m_ffi::ipc::prelude::*;

mod cli;
use cli::CliArgs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();
    println!("User Options: {args:?}");

    let mut ipc = IpcMaster::new_bind_on(&args.name);

    match args.cmd {
        cli::Command::Read { addr, size_b } => {
            // Issue read request
            let rd_ack = ipc.b_req_ack(IpcReq::Read {
                addr,
                size_b: size_b as usize,
            })?;
            println!("Read response {rd_ack:x?}");
            Ok(())
        }
        cli::Command::Write {
            addr,
            size_b,
            pattern,
        } => {
            // Issue write request
            let mut data = Vec::with_capacity(size_b as usize);
            if let Some(pat) = pattern {
                let pat_u8 = pat.to_ne_bytes();
                for _ in 0..(size_b / pat_u8.len() as u64) {
                    data.extend_from_slice(&pat_u8);
                }
            } else {
                // Randomly generate data
                let mut rng: StdRng = SeedableRng::from_os_rng();
                for _ in 0..size_b {
                    data.push(rng.r#gen::<u8>());
                }
            }

            // Issue write request
            let wr_ack = ipc.b_req_ack(IpcReq::Write {
                addr,
                data: ipc_channel::ipc::IpcSharedMemory::from_bytes(&data),
            })?;
            println!("Write response {wr_ack:x?}");
            Ok(())
        }
    }
}
