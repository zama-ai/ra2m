//! TraceParser
//! Convert Ra2m trace file in parquet file for post-mortem analysis

use polars::prelude::*;
use ra2m_sim::prelude::*;

use parsers::TraceKind;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Define CLI arguments
use clap::Parser;
#[derive(clap::Parser, Debug, Clone)]
#[clap(long_about = "Ra2m trace parser")]
pub struct Args {
    /// Trace file to convert
    #[clap(long, value_parser)]
    pub file: String,

    /// Expected trace kind
    #[clap(long, value_parser)]
    pub kind: TraceKind,
}

fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    println!("User Options: {args:?}");

    // Register tracing subscriber that use env-filter
    // Select verbosity with env_var: e.g. `RUST_LOG=Alu=trace`
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .compact()
        // Display source code file paths
        .with_file(false)
        // Display source code line numbers
        .with_line_number(false)
        .without_time()
        // Build & register the subscriber
        .init();

    match args.kind {
        TraceKind::MemBus => {
            let data = parsers::read_trace::<protocol::membus::MemBus>(&args.file)?;
            // Convert in polars-df
            let data_df = data
                .iter()
                .map(|(p, d)| {
                    let generic_map = Trace::export_as_traceable_map(d);
                    let df = parsers::to_dataframe(generic_map)?;
                    Ok((p, df))
                })
                .collect::<Result<HashMap<_, _>, parsers::ParserError>>()?;

            // Store it in parquet file for later analyses
            let file_p = Path::new(&args.file);
            let parent_p = file_p.parent();
            let file_stem = file_p.file_stem().unwrap().to_string_lossy();

            for (p, df) in data_df.iter() {
                // Save to parquet
                let out_f = format!("{}_{p}.parquet", file_stem);
                let out_p = match parent_p {
                    Some(p) => p.join(out_f),
                    None => PathBuf::from(out_f),
                };

                let mut file = std::fs::File::create(&out_p)?;
                ParquetWriter::new(&mut file).finish(&mut df.clone())?;
                tracing::info!("Saved data for {p} in: {}", out_p.display());

                // // Save to perfetto
                // let out_f = format!("{file_stem}_{p}.perfetto.json");
                // let out_p = match parent_p {
                //     Some(p) => p.join(out_f),
                //     None => PathBuf::from(out_f),
                // };
                // perfetto::from_dataframe(out_f, None)?;
            }
        }
        TraceKind::DmaBus => {
            todo!()
        }
    }
    Ok(())
}
