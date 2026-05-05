//! Direct Memory Access component
//! Convert DMA protocol in bi-directional MemBus access
//!

use protocol::{
    addr::Addr,
    dma::{DmaBus, DmaBusError},
    membus::{Command, MemBus},
    Mode,
};

use ra2m_sim::prelude::*;

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct DmaParams {
    pub inflight_req: usize,

    // Latencies governing the time taken by the Dma to construct MemBus request
    // It's use on both way DmaBus -> MemBus and MemBus -> DmaBus
    pub frontend_latency: types::Latency,

    /// Forward latency is the latency involved once a decision is made to
    /// forward the Read response to Write Request.
    pub forward_latency: types::Latency,

    /// Dma bandwidth
    /// Number of bits handled every cycles during forward
    /// (i.e when Dma received MemBus Read Resp and convert to MemBus Write Req)
    pub bandwidth: unit::BW,
}

/// Store internal state of Dma module
struct DmaInner {
    avail_id: Vec<usize>,
    pkt_store: Vec<Option<Packet<DmaBus<Addr>>>>,
}

impl DmaInner {
    fn new(params: &DmaParams) -> Self {
        Self {
            avail_id: (0..params.inflight_req).collect::<Vec<_>>(),
            pkt_store: (0..params.inflight_req).map(|_| None).collect::<Vec<_>>(),
        }
    }
}

#[derive(Module)]
pub struct Dma {
    params: DmaParams,
    props: Arc<module::Properties>,
    /// inbound: Received control request
    #[port]
    inbound: port::ReqRespPort<DmaBus<Addr>>,
    /// outbound: Send data request
    #[port]
    outbound: port::ReqRespPort<MemBus>,
    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,

    inner: Mutex<DmaInner>,
}

#[default_teardown]
impl Dma {
    pub fn new(params: DmaParams, props: module::Properties) -> Self {
        let props = Arc::new(props);

        Self {
            inbound: port::ReqRespPort::new("inbound", props.clone(), None, None),
            outbound: port::ReqRespPort::new(
                "outbound",
                props.clone(),
                Some(params.inflight_req),
                None,
            ),
            prc: Mutex::new(Vec::new()),
            inner: Mutex::new(DmaInner::new(&params)),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();

        // Request layers
        // Convert DmaBus into MemBus request
        let asc = self.clone();
        prc.push(spawn_prc!(Self::request_layer(asc)));

        // Forward layers
        // Forward MemBus Read response into MemBus Write request
        let asc = self.clone();
        prc.push(spawn_prc!(Self::forward_layer(asc)));
    }

    /// Compute Dma delay on frontend path
    fn frontend_delay(&self) -> time::Tick {
        self.params
            .frontend_latency
            .into_tick(self.properties().clock_domain())
    }

    /// Compute Dma delay on forward path
    fn forward_delay(&self, size: unit::Data) -> time::Tick {
        self.params
            .forward_latency
            .into_tick(self.properties().clock_domain())
            + time::Tick::from(size / self.params.bandwidth)
    }

    /// Construct MemBus request based on DmaBus request
    /// Request is then send to the outbound port
    async fn request_layer(self: Arc<Self>) {
        loop {
            let free_slot = {
                let inner = self.inner.lock().unwrap();
                inner.avail_id.len()
            };

            if free_slot == 0 {
                // No slot available, wait for slot freed event and retry
                event::Event::wait(&forge_event_name!(|self| "slot_freed")).await;
                continue;
            }

            // Take a slot in the pkt_store
            let token = {
                let mut inner = self.inner.lock().unwrap();
                inner.avail_id.pop().expect("Error with slot management")
            };

            // Wait for a Dma request
            let dma_pkt = self.inbound.rx().wait_pkt().await;

            // Construct request
            let mut mem_pkt = {
                let dma = dma_pkt.payload();
                MemBus::new_wrapped(
                    self.props.uid(),
                    Command::Read,
                    *dma.read_from(),
                    *dma.pattern(),
                    None,
                    None,
                )
            };

            // Store dma_pkt in pkt_store
            {
                let mut inner = self.inner.lock().unwrap();
                inner.pkt_store[token] = Some(dma_pkt);
            }

            // Append frontend delay and register pkt_store id in xbar_port list
            // In will be used to attach resp to associated dma_pkt during forward
            mem_pkt.append_delay(self.frontend_delay());
            mem_pkt.payload_mut().xbar_port_mut().push(token);
            self.outbound
                .tx()
                .send_pkt(mem_pkt)
                .await
                .expect("Faulty Tx access");
        }
    }

    /// Forward data from read target into write target
    async fn forward_layer(self: Arc<Self>) {
        loop {
            // Wait for a MemBusResp
            let mut mem_pkt = self.outbound.rx().wait_pkt().await;

            // Found associated entry
            let token = mem_pkt
                .payload_mut()
                .xbar_port_mut()
                .pop()
                .expect("Received unmatch MemBus response");

            let mem_pld = mem_pkt.payload_mut();

            match mem_pld.mode() {
                Mode::Response => match mem_pld.cmd() {
                    Command::Read => {
                        {
                            // Update pkt with inner info
                            let inner = self.inner.lock().unwrap();
                            let dma_pkt = inner.pkt_store[token].as_ref().unwrap();
                            // Update mem_pkt
                            // -> Forge a Write Req from Read Resp
                            mem_pld.set_mode(Mode::Request);
                            mem_pld.set_cmd(Command::Write);
                            mem_pld.set_addr(*dma_pkt.payload().write_to());
                            mem_pld.update_subrange();
                            // Store token for return path
                            mem_pld.xbar_port_mut().push(token);
                        }
                        let xfer_size = mem_pld.pattern().len();
                        mem_pkt.append_delay(self.forward_delay(xfer_size));

                        // Send it back through outbound port
                        self.outbound
                            .tx()
                            .send_pkt(mem_pkt)
                            .await
                            .expect("Faulty Tx access");
                    }
                    Command::Write => {
                        // Dma xfer is finished
                        let dma_pkt = {
                            // Update pkt with inner info
                            let mut inner = self.inner.lock().unwrap();
                            // 1. extract pkt at token position
                            // Add replacing element at end and swap remove
                            // Release associated slot
                            inner.pkt_store.push(None);
                            let mut dma_pkt = inner
                                .pkt_store
                                .swap_remove(token)
                                .expect("Error in pkt_store management");
                            inner.avail_id.push(token);

                            // 2. Update mode
                            dma_pkt.payload_mut().set_mode(Mode::Response);
                            dma_pkt
                        };

                        // 3. Move upward
                        self.inbound.tx().fwd_pkt(dma_pkt).await;
                    }
                    Command::AddrRange => panic!("Received AddrRange on Dma outbound interface"),
                },
                Mode::Error(mem_bus_error) => {
                    // Error occurred report to dma initiator
                    let dma_pkt = {
                        // Update pkt with inner info
                        let mut inner = self.inner.lock().unwrap();
                        // 1. extract pkt at token position
                        // Add replacing element at end and swap remove
                        // Release associated slot
                        inner.pkt_store.push(None);
                        let mut dma_pkt = inner
                            .pkt_store
                            .swap_remove(token)
                            .expect("Error in pkt_store management");
                        inner.avail_id.push(token);

                        // 2. Update mode
                        dma_pkt
                            .payload_mut()
                            .set_mode(Mode::Error(DmaBusError::Inner(Arc::new(
                                mem_bus_error.clone().into(),
                            ))));
                        dma_pkt
                    };

                    // 3. Move upward
                    self.inbound.tx().fwd_pkt(dma_pkt).await;
                }
                Mode::Request => {
                    panic!("Received Request on Dma outbound interface");
                }
            }
        }
    }
}
#[cfg(test)]
mod units_tests_dma {
    use super::*;
    use crate::mem;
    use crate::test::Agent;
    use ra2m_sim::prelude::{
        port::ReqRespPort,
        protocol::addr::{Addr, Pattern},
        protocol::membus::Command,
    };

    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn test_dma() -> Result<(), anyhow::Error> {
        Output::init("/tmp/ra2m/ra2m_cpn/integration_tests/test_dma");
        // Create global simulation state and custom scheduler for hardware task
        let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

        // Instantiate and bind module
        let mut root = module::Area::new(module::Properties::new(
            "root".to_owned(),
            Default::default(),
        ));

        // Xbar ===============================================================
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

        // NpRam ==============================================================
        root.insert_module(Arc::new(mem::NpRam::new(
            mem::NpRamParams {
                ports: 2, // 1 for Dma, 1 for Agent
                size: 10.MB(),
                base_addr: Some(0),
                latency: types::Latency::Cycle(1.cycles()),
                bandwidth: 1.GB_s(),
                binfile: None,
            },
            root.child_properties("ram_src", Default::default()),
        )));
        root.inner_bind("xbar::outbound", "ram_src::resp_port")?;

        root.insert_module(Arc::new(mem::NpRam::new(
            mem::NpRamParams {
                ports: 2, // 1 for Dma, 1 for Agent
                size: 10.MB(),
                base_addr: Some(0x1000000),
                latency: types::Latency::Cycle(1.cycles()),
                bandwidth: 1.GB_s(),
                binfile: None,
            },
            root.child_properties("ram_dst", Default::default()),
        )));
        root.inner_bind("xbar::outbound", "ram_dst::resp_port")?;

        // Dma ================================================================
        root.insert_module(Arc::new(Dma::new(
            DmaParams {
                inflight_req: 4,
                frontend_latency: types::Latency::Cycle(1.cycles()),
                forward_latency: types::Latency::Cycle(1.cycles()),
                bandwidth: 1.GB_s(),
            },
            root.child_properties("dma", Default::default()),
        )));
        root.inner_bind("dma::outbound", "xbar::inbound")?;

        // Agent ==============================================================
        let ram_agent_src = Arc::new(Agent::<ReqRespPort<MemBus>>::new(
            root.child_properties("ram_agent_src", Default::default()),
        ));
        root.insert_module(ram_agent_src.clone());
        root.inner_bind("ram_agent_src::port", "ram_src::resp_port")?;

        let ram_agent_dst = Arc::new(Agent::<ReqRespPort<MemBus>>::new(
            root.child_properties("ram_agent_dst", Default::default()),
        ));
        root.insert_module(ram_agent_dst.clone());
        root.inner_bind("ram_agent_dst::port", "ram_dst::resp_port")?;

        let dma_agent = Arc::new(Agent::<ReqRespPort<DmaBus<Addr>>>::new(
            root.child_properties("dma_agent_dst", Default::default()),
        ));
        root.insert_module(dma_agent.clone());
        root.inner_bind("dma_agent_dst::port", "dma::inbound")?;

        // Init modules ======================================================
        let root = Arc::new(root);
        root.init();

        // Test Body =========================================================
        spawn_prc!(async {
            event::Event::wait("AddrRangeMap_rdy").await;

            // Gen pattern data and write in ram_src
            let data = vec![0xfa; 2048];
            let _ = ram_agent_src
                .clone()
                .b_req_resp_flush(MemBus::new(
                    &0,
                    Command::Write,
                    Addr::Phys(0),
                    Pattern::Simple(2048.Byte()),
                    Some(data.as_slice()),
                ))
                .await
                .unwrap();

            // Issued Dma xfer from src to dst
            let _ = dma_agent
                .clone()
                .b_req_resp_flush(DmaBus::new(
                    Addr::Phys(0),
                    Addr::Phys(0x1000000),
                    Pattern::Simple(2048.Byte()),
                ))
                .await
                .unwrap();
            // Read back value in dest
            let resp = ram_agent_dst
                .clone()
                .b_req_resp_flush(MemBus::new(
                    &0,
                    Command::Read,
                    Addr::Phys(0),
                    Pattern::Simple(2048.Byte()),
                    None,
                ))
                .await
                .unwrap();
            // check content
            assert_eq!(
                data.as_slice(),
                resp.data().as_slice(),
                "Error: Src/Dst data mismatch"
            );
            event::Event::wait("EndOfSimulation").await;
        });

        // Start scheduler
        sched.simulate(10.us().into()).await;
        event::Event::triggered("EndOfSimulation", None);
        Ok(())
    }
}
