use ra2m_cpn::{mem, test};
use ra2m_sim::prelude::*;

use clap::Parser;

use std::sync::Arc;
use tokio::sync::mpsc;

/// Define CLI arguments
#[derive(clap::Parser, Debug, Clone)]
#[clap(long_about = "Rust Async Architecture Modelling")]
pub struct Args {
    /// Simulation timing mode
    /// Could be:
    ///  * LooselyTimed      : [LT, lt, LooselyTimed, Loosely]
    ///  * ApproximatelyTimed: [AT, at, ApproxTimed, Approx]
    #[clap(long, value_parser, default_value = "AT")]
    timing_mode: time::TimingMode,

    /// Starting simulation tick
    #[clap(long, value_parser, default_value_t = 0)]
    tick: usize,

    /// Simulation duration [val_unit]
    #[clap(long, value_parser, default_value = "1_us")]
    duration: unit::Time,

    /// Number of tick per second (resolution) [val_unit]
    #[clap(long, value_parser, default_value = "1_ps")]
    timescale: unit::Time,

    /// Max number of Hardware thread that could be handled
    #[clap(long, value_parser, default_value_t = 0x1000)]
    max_hw_process: usize,

    /// Number of Traffic generator to instantiate
    #[clap(long, value_parser, default_value_t = 4)]
    traffic_gen: usize,

    /// Number of Memory chunk to instantiate
    #[clap(long, value_parser, default_value_t = 4)]
    mem_chunk: usize,

    /// Size of Memory chunk
    #[clap(long, value_parser, default_value = "20_MB")]
    mem_chunk_size: unit::Data,

    /// Memory access kind in percentages
    #[clap(long, value_parser, default_value_t = 50)]
    write_rate: usize,

    /// Tweak component logging
    /// Provide list regex=dflt_verb:{cat:verb, ...} to alter component log_filter
    #[clap(long, value_parser)]
    log_args: Option<Vec<log::Args>>,

    // Tweak component tracing
    /// Provide list regex={cat:verb, ...} to alter component hw_tracer
    #[clap(long, value_parser)]
    trace_args: Option<Vec<trace::Args>>,

    /// Global log Verbosity
    #[clap(long, value_parser)]
    dflt_log_verb: Option<log::Verbosity>,
}

/// Core example function
/// Wrap in main later to enable single/mt -thread configuration
async fn addr_map() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    println!("User Options: {args:?}");

    Output::init("/tmp/ra2m/examples/addr_map");

    // Create global simulation state and custom scheduler for hardware task
    let mut sched = init_simulation(
        args.tick,
        args.timescale,
        args.timing_mode,
        args.max_hw_process,
    );

    // Instantiate and bind module
    let mut root = module::Area::new(module::Properties::new(
        "root".to_owned(),
        Default::default(),
    ));
    // Xbar ==============================================================
    root.insert_module(Arc::new(mem::XBar::new(
        mem::XBarParams {
            inflight_req: 10,
            frontend_latency: types::Latency::Cycle(2.cycles()),
            forward_latency: types::Latency::Cycle(1.cycles()),
            bandwidth: 10.MiB_s(),
            inbound_cap: Some(args.traffic_gen),
            outbound_cap: None,
        },
        root.child_properties("xbar", Default::default()),
    )));

    // NpRam =======================================================
    for i in 0..args.mem_chunk {
        let name = format!("mem_{i}");
        // create module
        root.insert_module(Arc::new(mem::NpRam::new(
            mem::NpRamParams {
                ports: 1,
                size: args.mem_chunk_size,
                base_addr: Some((args.mem_chunk_size * i).into()),
                latency: types::Latency::Time {
                    avg: 5.ns(),
                    var: 1.ns(),
                },
                bandwidth: 10.MiB_s(),
                binfile: None,
            },
            root.child_properties(&name, Default::default()),
        )));

        // Attach to Xbar
        let mem_port = name + "::resp_port";
        root.inner_bind("xbar::outbound", &mem_port)?;
    }

    // Traffic =======================================================
    let (check_tx, check_rx) = mpsc::channel(400); // TODO
    let addr_space = args.mem_chunk_size * args.mem_chunk;
    let write_rate = std::cmp::max(args.write_rate, 100);
    for i in 0..args.traffic_gen {
        let name = format!("traffic_gen_{i}");
        // create module
        root.insert_module(Arc::new(test::TrafficGen::new(
            test::TrafficGenParams {
                addr_range: (0, addr_space),
                addr_chunk: args.mem_chunk_size,
                data_range: (1.Byte(), 1.KiB()),
                read_write_weight: (100 - write_rate, write_rate),
                inflight_req: 10,
                rng_seed: Some(i as u64),
            },
            root.child_properties(&name, Default::default()),
            check_tx.clone(),
        )));

        // Attach to Xbar
        let tg_port = name + "::req_port";
        root.inner_bind("xbar::inbound", &tg_port)?;
    }

    // MemChecker =========================================================
    let _mem_check_handler = test::MemoryChecker::start(
        test::MemoryCheckerParams {
            offset: 0,
            size: addr_space,
            binfile: None,
        },
        check_rx,
    );

    // Init modules ======================================================
    let root = Arc::new(root);
    user_log_args(&root, args.log_args);
    user_trace_args(&root, args.trace_args);
    if let Some(dflt_verb) = args.dflt_log_verb {
        Output::log_update(true, Some(log::LogFilter::new(dflt_verb)));
    }
    root.clone().init();

    // Start scheduler
    sched.simulate(args.duration.into()).await;
    Ok(())
}

#[cfg(not(feature = "tokio-mt"))]
#[tokio::main(worker_threads = 1)]
async fn main() -> Result<(), anyhow::Error> {
    println!("Use Single-threaded Tokio runtime [not feature: \"tokio-mt\"]");
    addr_map().await?;
    Ok(())
}

#[cfg(feature = "tokio-mt")]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    println!("Use multi-threaded Tokio runtime [feature: \"tokio-mt\"]");
    addr_map().await?;
    Ok(())
}
