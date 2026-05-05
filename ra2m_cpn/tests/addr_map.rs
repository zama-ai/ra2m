use ra2m_cpn::{mem, test};
use ra2m_sim::prelude::*;

use serial_test::serial;
use std::sync::Arc;
use tokio::sync::mpsc;

async fn test_addr_map_with_error() -> Result<(), anyhow::Error> {
    Output::init("/tmp/ra2m/ra2m_cpn/integration_tests/addr_map");

    // Create global simulation state and custom scheduler for hardware task
    let mut sched = init_simulation(0, 1.ps(), time::TimingMode::AT, 400);

    // Create module properties/parameters
    let mem_size = 10.MB();
    // NpRam =======================================================
    let mema_props = module::Properties::new("root.memA".to_owned(), Default::default());

    let mem_a = Arc::new(mem::NpRam::new(
        mem::NpRamParams {
            ports: 1,
            size: mem_size,
            base_addr: Some(0),
            latency: types::Latency::Time {
                avg: 5.ns(),
                var: 1.ns(),
            },
            bandwidth: 1.MB_s(),
            binfile: None,
        },
        mema_props,
    ));

    let memb_props = module::Properties::new("root.memB".to_owned(), Default::default());
    let mem_b = Arc::new(mem::NpRam::new(
        mem::NpRamParams {
            ports: 1,
            size: mem_size,
            base_addr: Some(mem_size.into()),
            latency: types::Latency::Time {
                avg: 5.ns(),
                var: 1.ns(),
            },
            bandwidth: 1.MB_s(),
            binfile: None,
        },
        memb_props,
    ));

    // Xbar ==============================================================
    let xbar_props = module::Properties::new("root.xbar".to_owned(), Default::default());
    let xbar = Arc::new(mem::XBar::new(
        mem::XBarParams {
            inflight_req: 1,
            frontend_latency: types::Latency::Cycle(2.cycles()),
            forward_latency: types::Latency::Cycle(1.cycles()),
            bandwidth: 1.MB_s(),
            inbound_cap: None,
            outbound_cap: None,
        },
        xbar_props,
    ));

    // TrafficGen ========================================================
    let (check_tx, check_rx) = mpsc::channel(400); // TODO

    let tga_props = module::Properties::new("root.traffic_genA:".to_owned(), Default::default());
    let traffic_gen_a = Arc::new(test::TrafficGen::new(
        test::TrafficGenParams {
            addr_range: (0, mem_size * 2),
            addr_chunk: mem_size,
            data_range: (1.Byte(), 1.KiB()),
            read_write_weight: (50, 50),
            inflight_req: 1,
            rng_seed: Some(0),
        },
        tga_props,
        check_tx.clone(),
    ));

    let tgb_props = module::Properties::new("root.traffic_genB:".to_owned(), Default::default());
    let traffic_gen_b = Arc::new(test::TrafficGen::new(
        test::TrafficGenParams {
            addr_range: (0, (mem_size * 2)),
            addr_chunk: mem_size,
            data_range: (1.Byte(), 1.KiB()),
            read_write_weight: (50, 50),
            inflight_req: 1,
            rng_seed: Some(1),
        },
        tgb_props,
        check_tx.clone(),
    ));

    // MemChecker =========================================================
    let checker_params = test::MemoryCheckerParams {
        offset: 0,
        size: mem_size * 2,
        binfile: None,
    };
    let _mem_check_handler = test::MemoryChecker::start(checker_params, check_rx);

    // Bind ports ========================================================
    xbar.port("inbound")
        .bind(traffic_gen_a.port("req_port").into())?;
    traffic_gen_b
        .port("req_port")
        .bind(xbar.port("inbound").into())?;
    xbar.port("outbound").bind(mem_a.port("resp_port").into())?;
    xbar.port("outbound").bind(mem_b.port("resp_port").into())?;

    // Init modules ======================================================
    traffic_gen_a.init();
    traffic_gen_b.init();
    xbar.init();
    mem_a.init();
    mem_b.init();

    // Start scheduler
    sched.simulate(100.us().into()).await;
    Ok(())
}

#[tokio::main]
#[test] // NB: Use of expected macro #[tokio::test] failed due to nested crates
#[serial]
async fn test_addr_map() {
    if let Err(err) = test_addr_map_with_error().await {
        panic!("test_addr_map encounter error {err}");
    }
}
