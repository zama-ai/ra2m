use rand::{
    rngs::StdRng,
    {Rng, SeedableRng},
};

use ra2m_ffi::rpc::prelude::*;
mod cli;
use clap::Parser;
use cli::CliArgs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();
    println!("User Options: {args:?}");

    let mut ep = RemotePortEndpoint::new(args.name.as_str()).await?;
    match args.cmd {
        cli::Command::Read { addr, size_b } => {
            let data = ep.rp_read(addr, size_b).await?;
            println!("Read response {data:x?}");
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

            ep.rp_write(addr, data).await?;
            println!("Write response success");
            Ok(())
        }
    }
}
