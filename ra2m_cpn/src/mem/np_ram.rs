//! Multi-Port RAM
//! RAM Memory model with multiple ReqRespPort based on MemoryBus protocol.
//! Use multiple response layer to handle multiple requests
//!

use protocol::{
    addr::{Addr, Pattern, SubRangeAddr},
    membus::{Command, MemBus, MemBusError},
    Mode,
};
use ra2m_sim::prelude::*;

use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct NpRamParams {
    pub ports: usize,
    pub size: unit::Data,
    pub base_addr: Option<usize>,

    pub latency: types::Latency,
    pub bandwidth: unit::BW,
    pub binfile: Option<String>,
}

#[derive(Module)]
pub struct NpRam {
    params: NpRamParams,
    props: Arc<module::Properties>,
    #[port]
    resp_port: port::PortVec<port::ReqRespPort<MemBus>>,
    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    memory: Mutex<SparseVec<u8>>,
}

#[default_teardown]
impl NpRam {
    pub fn new(params: NpRamParams, props: module::Properties) -> Self {
        let size: usize = (&params.size).into();

        // Allocate underlying memory
        // Used sparse container to prevent high-ram usage
        let mut mem = SparseVec::new(size).expect("Unable to create memory container");

        // Init content from binary file or default init
        if let Some(binfile) = &params.binfile {
            let file_mem = std::fs::read(binfile)
                .unwrap_or_else(|err| panic!("Failed to open file {binfile} with error {err}"));
            mem.init_from_vec(file_mem)
                .expect("Binfile content doesn't fit in memory");
        }

        let props = Arc::new(props);
        NpRam {
            resp_port: port::PortVec::new(params.ports, "resp", props.clone(), None, None),
            prc: Mutex::new(Vec::new()),
            memory: Mutex::new(mem),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        // Start response layer for each port and register associated range if needed
        // NB: every port see the same addr_range
        let port_len = self.resp_port.len();
        let base_addr = self.params.base_addr;
        for idx in 0..port_len {
            if let Some(offset) = base_addr {
                // NpRam only support one contiguous addr_range
                let asc = self.clone();
                let addr_range = vec![Addr::Range(offset, self.params.size)];
                prc.push(spawn_prc!(async {
                    match protocol::membus::addr_range_register(
                        asc.props.uid(),
                        &asc.resp_port[idx],
                        &addr_range,
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(err) => {
                            log!(|asc| log::Category::Protocol, log::Verbosity::Warning => err );
                        }
                    }
                }));
            }

            let asc = self.clone();
            prc.push(spawn_prc!(Self::response_layer(asc, idx)));
        }
    }

    fn handle_request(self: Arc<Self>, req: &mut MemBus) -> Option<time::Tick> {
        let mut delay: time::Tick = Default::default();

        // Register as request handler
        req.trace_mut()
            .push(types::Handler::base(*self.properties().uid()));

        // Compute frontend delay
        // => Time required to decode the address and access the internal memory array
        delay += self
            .params
            .latency
            .into_tick(self.properties().clock_domain());

        // Decode address
        let addr = match req.subrange_addr() {
            SubRangeAddr::Phys(t) => *t,
            _ => {
                req.set_mode(Mode::Error(MemBusError::SubRange(*req.subrange_addr())));
                return Some(delay);
            }
        };

        // Decode access pattern and command
        // In case of error, set mode field accordingly
        match req.pattern() {
            Pattern::Simple(d) => {
                match req.cmd() {
                    Command::Read => {
                        let len: usize = d.into();
                        let mem_slice = &self.memory.lock().unwrap()[addr..(addr + len)];
                        req.data_mut().extend_from_slice(mem_slice);
                    }
                    Command::Write => {
                        let len = d.into();
                        let req_slice = req.data().as_slice();
                        assert_eq!(
                            req_slice.len(),
                            len,
                            "Provided data length didn't match with Pattern"
                        );
                        let mut mem = self.memory.lock().unwrap();
                        mem[addr..(addr + len)].copy_from_slice(&req_slice[..len]);
                    }
                    _ => {
                        req.set_mode(Mode::Error(MemBusError::Cmd(*req.cmd())));
                        return Some(delay);
                    }
                };
            }
            _ => {
                req.set_mode(Mode::Error(MemBusError::Pattern(*req.pattern())));
                return Some(delay);
            }
        }

        // Compute access delay
        // => Time requeired to stream data out of memory array
        delay += time::Tick::from(req.pattern().len() / self.params.bandwidth);

        // Everything goes well, set success status and return
        req.set_mode(Mode::Response);
        Some(delay)
    }

    async fn response_layer(self: Arc<Self>, idx: usize) {
        loop {
            match self.resp_port[idx]
                .wait_req_forge_resp(|req| self.clone().handle_request(req))
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    log!(|self| log::Category::Protocol, log::Verbosity::Warning => err );
                }
            };
        }
    }
}

#[cfg(test)]
mod units_tests_np_ram {
    use super::*;
    use crate::test;

    use serial_test::serial;
    use tokio::sync::mpsc;

    async fn test_np_lram(ports: usize) {
        let status = test_np_ram(
            ports,
            NpRamParams {
                ports,
                size: 10.MB(),
                base_addr: None,
                latency: types::Latency::Time {
                    avg: 5.ns(),
                    var: 1.ns(),
                },
                bandwidth: 1.GB_s(),
                binfile: None,
            },
        )
        .await;

        if let Err(err) = status {
            panic!("test_np_lram encounter error {err}");
        }
    }

    async fn test_np_cram(ports: usize) {
        let status = test_np_ram(
            ports,
            NpRamParams {
                ports,
                size: 10.MB(),
                base_addr: None,
                latency: types::Latency::Cycle(1.cycles()),
                bandwidth: 1.GB_s(),
                binfile: None,
            },
        )
        .await;

        if let Err(err) = status {
            panic!("test_np_lram encounter error {err}");
        }
    }

    async fn test_np_ram(ports: usize, params: NpRamParams) -> Result<(), anyhow::Error> {
        // Create global simulation state and custom scheduler for hardware task
        let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

        // NpRam ==============================================================
        let mem_props = module::Properties::new("root.np_ram".to_owned(), Default::default());
        let np_ram = Arc::new(NpRam::new(params.clone(), mem_props));

        // Traffic =======================================================
        let (check_tx, check_rx) = mpsc::channel(400);
        let mut traffic_gen_module: Vec<Arc<dyn Module>> = Vec::new();

        // Isolate TG to prevent check error on unordered access
        let tg_chunk = params.size / ports;
        for idx in 0..ports {
            let traffic_gen = Arc::new(test::TrafficGen::new(
                test::TrafficGenParams {
                    addr_range: ((tg_chunk * idx).into(), tg_chunk),
                    addr_chunk: params.size,
                    data_range: (1.Byte(), 1.KiB()),
                    read_write_weight: (50, 50),
                    inflight_req: 10,
                    rng_seed: Some(idx as u64),
                },
                module::Properties::new(format!("root.traffic_gen_{idx}"), Default::default()),
                check_tx.clone(),
            ));

            // Attach to np_ram
            np_ram
                .port("resp_port")
                .bind(traffic_gen.port("req_port").into())?;

            // Push into vec for later init
            traffic_gen_module.push(traffic_gen);
        }

        // MemChecker =========================================================
        let _mem_check_handler = test::MemoryChecker::start(
            test::MemoryCheckerParams {
                offset: 0,
                size: params.size,
                binfile: None,
            },
            check_rx,
        );

        // Init modules ======================================================
        // Clone them for later logFilter update
        for tg in &traffic_gen_module {
            tg.clone().init();
        }
        np_ram.clone().init();

        // Start scheduler
        sched.simulate(10.us().into()).await;
        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn test_sp_ram() {
        test_np_lram(1).await;
        test_np_cram(1).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_dp_ram() {
        test_np_lram(2).await;
        test_np_cram(2).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_qp_ram() {
        test_np_lram(4).await;
        test_np_cram(4).await;
    }
}
