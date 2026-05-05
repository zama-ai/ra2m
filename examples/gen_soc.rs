use ra2m_cpn::mem;
use ra2m_sim::prelude::*;

use ra2m_cpn::ffi_bridge;

use std::sync::Arc;

use clap::Parser;
/// Define CLI arguments
#[derive(clap::Parser, Debug, Clone)]
#[clap(long_about = "Ra2m Generic Soc Model")]
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

    /// Number of Memory chunk to instantiate
    #[clap(long, value_parser, default_value_t = 4)]
    ram_chunk: usize,

    /// Size of Memory chunk
    #[clap(long, value_parser, default_value = "256_MB")]
    ram_chunk_size: unit::Data,

    /// Enable ffi with the given path
    /// NB: by default rely on IPC but could use RPC with `rpc` features.
    /// With Ipc, any filename could be used
    /// With Rpc, a valid Tcp socket must be specified (e.g 127.0.0.1:8090 for localhost with port 8090)
    #[clap(long, value_parser)]
    ffi_path: Option<String>,

    // TODO Add configuration for processor
    // A Generic SoC should have Cpu core
    /// Tweak component logging
    /// Provide list regex=dflt_verb:{cat:verb, ...} to alter component log_filter
    #[clap(long, value_parser)]
    log_args: Option<Vec<log::Args>>,

    // Tweak component tracing
    /// Provide list regex={cat:verb, ...} to alter component hw_tracer
    #[clap(long, value_parser)]
    trace_args: Option<Vec<trace::Args>>,
}

/// Core example function
/// Wrap in main later to enable single/mt -thread configuration
async fn addr_map() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    println!("User Options: {args:?}");

    Output::init("/tmp/ra2m/examples/gen_soc");

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
            inbound_cap: None,
            outbound_cap: None,
        },
        root.child_properties("xbar", Default::default()),
    )));

    // NpRam =======================================================
    for i in 0..args.ram_chunk {
        let name = format!("mem_{i}");
        // create module
        root.insert_module(Arc::new(mem::NpRam::new(
            mem::NpRamParams {
                ports: 1,
                size: args.ram_chunk_size,
                base_addr: Some((args.ram_chunk_size * i).into()),
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

    // Host RPC ==========================================================
    if let Some(ffi) = args.ffi_path {
        #[cfg(feature = "rpc")]
        {
            // Create Host to Sim bridge
            // H2s bridge could access the full range of Ram
            let addr_space = args.ram_chunk_size * args.ram_chunk;
            // create module
            root.insert_module(Arc::new(ffi_bridge::rpc::H2sBridge::new(
                ffi_bridge::rpc::H2sBridgeParams {
                    rpc_path: ffi.clone(),
                    addr_range: (0, addr_space),
                    inflight_req: 10,
                    keep_alive: Some(10.ms()),
                },
                root.child_properties("H2sBridge", Default::default()),
            )));

            // Attach to Xbar
            root.inner_bind("xbar::inbound", "H2sBridge::port")?;

            // Create Sim to Host bridge
            // S2h is accessible after Ram range (same width)
            let addr_space = args.ram_chunk_size * args.ram_chunk;
            // create module
            root.insert_module(Arc::new(ffi_bridge::rpc::S2hBridge::new(
                ffi_bridge::rpc::S2hBridgeParams {
                    rpc_path: ffi,
                    addr_range: Some((usize::from(addr_space), addr_space)),
                },
                root.child_properties("S2hBridge", Default::default()),
            )));

            // Attach to Xbar
            root.inner_bind("xbar::outbound", "S2hBridge::port")?;
        }
        #[cfg(not(feature = "rpc"))]
        {
            // Create Host to Sim bridge
            // H2s bridge could access the full range of Ram
            let addr_space = args.ram_chunk_size * args.ram_chunk;
            // create module
            root.insert_module(Arc::new(ffi_bridge::ipc::H2sBridge::new(
                ffi_bridge::ipc::H2sBridgeParams {
                    ipc_path: ffi.clone(),
                    addr_range: (0, addr_space),
                    inflight_req: 10,
                    keep_alive: Some(10.ms()),
                    polling_rate: 10.ms(),
                },
                root.child_properties("H2sBridge", Default::default()),
            )));

            // Attach to Xbar
            root.inner_bind("xbar::inbound", "H2sBridge::port")?;

            // Create Sim to Host bridge
            // S2h is accessible after Ram range (same width)
            let addr_space = args.ram_chunk_size * args.ram_chunk;
            // create module
            root.insert_module(Arc::new(ffi_bridge::ipc::S2hBridge::new(
                ffi_bridge::ipc::S2hBridgeParams {
                    ipc_path: ffi,
                    addr_range: Some((usize::from(addr_space), addr_space)),
                },
                root.child_properties("S2hBridge", Default::default()),
            )));

            // Attach to Xbar
            root.inner_bind("xbar::outbound", "S2hBridge::port")?;
        }
    }

    // Init modules ======================================================
    let root = Arc::new(root);
    user_log_args(&root, args.log_args);
    user_trace_args(&root, args.trace_args);
    root.clone().init();

    // Start scheduler
    let (tick, kind) = sched.simulate(args.duration.into()).await;
    println!("Simulation exit @{tick} from {kind:?}");
    root.teardown();
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
