//! Network capable Direct Memory Access component
//! Handle DmaProtocol across network boundaries
//!

use protocol::{
    addr::Addr,
    dma::{DmaBus, DmaBusError},
    membus::{Command, MemBus},
    Mode,
};

use ra2m_sim::prelude::{
    protocol::network::{Network, NetworkError},
    *,
};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct NDmaParams<T> {
    pub inflight_req: usize,

    /// Depict current NodeId from network point of view
    pub node_id: T,

    /// Latencies governing the time taken by the Dma to construct MemBus request
    /// It's use on both way DmaBus -> MemBus and MemBus -> DmaBus
    pub frontend_latency: types::Latency,

    /// Forward latency is the latency involved once a decision is made to
    /// forward the Read response to Write Request.
    pub forward_latency: types::Latency,

    /// Dma bandwidth
    /// Number of bits handled every cycles during forward
    /// (i.e when Dma received MemBus Read Resp and convert to MemBus Write Req)
    pub bandwidth: unit::BW,
}

/// Network Dma store
/// Could contain DmaBus packet (local request) or Network packet (external request)
#[derive(Debug)]
enum NDmaSlot<T>
where
    T: Clone,
    for<'a> &'a (T, Addr): Into<Traceable>,
{
    None,
    Dma(Box<Packet<DmaBus<(T, Addr)>>>),
    Net { from: T, to: T },
}

impl<T> NDmaSlot<T>
where
    T: Clone,
    for<'a> &'a (T, Addr): Into<Traceable>,
{
    fn as_dma_ref(&self) -> Option<&Packet<DmaBus<(T, Addr)>>> {
        match self {
            Self::Dma(val) => Some(val),
            _ => None,
        }
    }
    fn unwrap_dma(self) -> Box<Packet<DmaBus<(T, Addr)>>> {
        match self {
            Self::Dma(val) => val,
            _ => panic!("called `NDmaSlot::unwrap_dma()` on a invalid value"),
        }
    }

    fn unwrap_net(self) -> (T, T) {
        match self {
            Self::Net { from, to } => (from, to),
            _ => panic!("called `NnetSlot::unwrap_net()` on a invalid value"),
        }
    }
}

/// Network Dma Store Kind
/// Only here to enable clone and circumvent issue with borrow of !Send elem
/// across await boundaries
#[derive(Debug, Clone)]
enum NDmaSlotKind {
    None,
    Dma,
    Net,
}

impl<T> From<&NDmaSlot<T>> for NDmaSlotKind
where
    T: Clone,
    for<'a> &'a (T, Addr): Into<Traceable>,
{
    fn from(value: &NDmaSlot<T>) -> Self {
        match value {
            NDmaSlot::None => Self::None,
            NDmaSlot::Dma(_) => Self::Dma,
            NDmaSlot::Net { .. } => Self::Net,
        }
    }
}

/// Store internal state of Dma module
struct NDmaInner<T>
where
    T: Clone,
    for<'a> &'a (T, Addr): Into<Traceable>,
{
    avail_id: Vec<usize>,
    pkt_store: Vec<NDmaSlot<T>>,
}

impl<T> NDmaInner<T>
where
    T: Clone,
    for<'a> &'a (T, Addr): Into<Traceable>,
{
    fn new(params: &NDmaParams<T>) -> Self {
        Self {
            avail_id: (0..params.inflight_req).collect::<Vec<_>>(),
            pkt_store: (0..params.inflight_req)
                .map(|_| NDmaSlot::None)
                .collect::<Vec<_>>(),
        }
    }
}

#[derive(Module)]
pub struct NDma<T>
where
    T: 'static + Send + Sync + Clone + std::fmt::Debug + std::cmp::Eq + serde::Serialize,
    for<'a> &'a (T, Addr): Into<Traceable>,
    for<'a> &'a T: Into<Traceable>,
{
    params: NDmaParams<T>,
    props: Arc<module::Properties>,
    /// inbound: Received control request
    #[port]
    inbound: port::ReqRespPort<DmaBus<(T, Addr)>>,
    /// outbound: Send data request
    #[port]
    mem: port::ReqRespPort<MemBus>,
    #[port]
    net_inbound: port::ReqRespPort<Network<T, MemBus>>,
    #[port]
    net_outbound: port::ReqRespPort<Network<T, MemBus>>,

    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,

    inner: Mutex<NDmaInner<T>>,
}

#[default_teardown]
impl<T> NDma<T>
where
    T: 'static + Send + Sync + Clone + std::fmt::Debug + std::cmp::Eq + serde::Serialize,
    for<'a> &'a (T, Addr): Into<Traceable>,
    for<'a> &'a T: Into<Traceable>,
{
    pub fn new(params: NDmaParams<T>, props: module::Properties) -> Self {
        let props = Arc::new(props);

        Self {
            inbound: port::ReqRespPort::new("inbound", props.clone(), None, None),
            mem: port::ReqRespPort::new("mem", props.clone(), Some(params.inflight_req), None),
            net_inbound: port::ReqRespPort::new(
                "net_inbound",
                props.clone(),
                Some(params.inflight_req),
                None,
            ),
            net_outbound: port::ReqRespPort::new("net_outbound", props.clone(), None, None),
            prc: Mutex::new(Vec::new()),
            inner: Mutex::new(NDmaInner::new(&params)),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();

        // Request layers
        // Convert DmaBus into MemBus/Network request
        let asc = self.clone();
        prc.push(spawn_prc!(Self::request_layer(asc)));

        // Forward layers
        // Correctly dispatch local memory response
        let asc = self.clone();
        prc.push(spawn_prc!(Self::mem_layer(asc)));

        // Network layers
        // handle egress (i.e. master port)
        let asc = self.clone();
        prc.push(spawn_prc!(Self::egress_layer(asc)));
        // handle ingress traffic (i.e. slave port)
        let asc = self.clone();
        prc.push(spawn_prc!(Self::ingress_layer(asc)));
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

    /// Construct MemBus/Network request based on DmaBus request
    /// Request is then send to the outbound port
    async fn request_layer(self: Arc<Self>) {
        loop {
            // Wait for free slot in the pkt_store
            let token = self.wait_slot().await;

            // Wait for a Dma request, translate and forward to subport
            let dma_pkt = self.inbound.rx().wait_pkt().await;

            self.dma_to_inner_req(dma_pkt, token).await;
        }
    }

    /// Forward data from read target into write target
    /// Read target was reach through local mem port
    async fn mem_layer(self: Arc<Self>) {
        loop {
            // Wait for a MemBusResp
            let mem_pkt = self
                .mem
                .rx()
                .wait_pkt_ep(None)
                .await
                .expect("Error invalid packet");
            let inner_mode = mem_pkt.payload().mode().clone();

            match inner_mode {
                Mode::Response => self.forward_resp(mem_pkt).await,
                Mode::Error(membus_error) => {
                    self.forward_error(mem_pkt, Arc::new(membus_error.into()))
                        .await
                }
                Mode::Request => {
                    panic!("Received Request on NDma mem interface");
                }
            }
        }
    }

    /// Receive external Network request
    async fn ingress_layer(self: Arc<Self>) {
        loop {
            // Wait for free slot in the pkt_store
            let token = self.wait_slot().await;

            // Wait for Network packet
            let net_pkt = self.net_inbound.rx().wait_pkt().await;

            // Store net_pkt properties in pkt_store
            {
                let net = net_pkt.payload();
                let mut inner = self.inner.lock().unwrap();
                let ndma_slot = NDmaSlot::Net {
                    from: net.from().clone(),
                    to: net.to().clone(),
                };
                log!(|self| log::Category::Own, log::Verbosity::Trace  => token, ndma_slot);
                inner.pkt_store[token] = ndma_slot;
            }

            // Extract inner and issue on local memory interface
            let mut mem_pkt = net_pkt.inner_unwrap();
            // Update handler history
            mem_pkt
                .payload_mut()
                .append_handler(types::Handler::base(*self.properties().uid()));
            // Update xbar port stack for backward path
            mem_pkt.payload_mut().xbar_port_mut().push(token);

            self.mem
                .tx()
                .send_pkt(mem_pkt)
                .await
                .expect("Faulty Tx access");
        }
    }

    /// Handle response from egress traffic
    async fn egress_layer(self: Arc<Self>) {
        loop {
            // Wait for Network packet
            let net_pkt = self
                .net_outbound
                .rx()
                .wait_pkt_ep(None)
                .await
                .expect("Error invalid packet");

            let mode = net_pkt.payload().mode().clone();
            match mode {
                Mode::Response => {
                    let mem_pkt = net_pkt.inner_unwrap();
                    self.forward_resp(mem_pkt).await
                }
                Mode::Error(network_error) => {
                    let mem_pkt = net_pkt.inner_unwrap();
                    self.forward_error(mem_pkt, Arc::new(network_error.into()))
                        .await
                }
                Mode::Request => {
                    panic!("Received Request on NDma net_outbound interface");
                }
            }
        }
    }
}

impl<T> NDma<T>
where
    T: 'static + Send + Sync + Clone + std::fmt::Debug + std::cmp::Eq + serde::Serialize,
    for<'a> &'a (T, Addr): Into<Traceable>,
    for<'a> &'a T: Into<Traceable>,
{
    fn remove_slot_at(&self, pos: usize) -> NDmaSlot<T> {
        let mut inner = self.inner.lock().unwrap();
        // 1. extract pkt at token position
        // Add replacing element at end and swap remove
        // Release associated slot
        inner.pkt_store.push(NDmaSlot::None);
        let gen_pkt = inner.pkt_store.swap_remove(pos);
        inner.avail_id.push(pos);
        event::Event::triggered(&forge_event_name!(|self| "slot_freed"), None);
        gen_pkt
    }
    async fn wait_slot(&self) -> usize {
        loop {
            let free_slot = {
                let inner = self.inner.lock().unwrap();
                inner.avail_id.len()
            };

            if free_slot == 0 {
                // No slot available, wait for slot freed event and retry
                event::Event::wait(&forge_event_name!(|self| "slot_freed")).await;
            } else {
                break;
            }
        }

        // Take a slot in the pkt_store
        let mut inner = self.inner.lock().unwrap();
        inner.avail_id.pop().expect("Error with slot management")
    }

    async fn dma_to_inner_req(&self, dma_pkt: Packet<DmaBus<(T, Addr)>>, token: usize) {
        // Clone useful info
        let (read_from, pattern) = {
            let dma = dma_pkt.payload();
            (dma.read_from().clone(), *dma.pattern())
        };
        // Store dma_pkt in pkt_store
        {
            let mut inner = self.inner.lock().unwrap();
            let ndma_slot = NDmaSlot::Dma(Box::new(dma_pkt));
            log!(|self| log::Category::Own, log::Verbosity::Trace  => token, ndma_slot);
            inner.pkt_store[token] = ndma_slot;
        }

        // Construct and issue request
        if read_from.0 == self.params.node_id {
            // Local access
            let mut mem_pkt = MemBus::new_wrapped(
                self.props.uid(),
                Command::Read,
                read_from.1,
                pattern,
                None,
                None,
            );

            // Append frontend delay and register pkt_store id in xbar_port list
            // In will be used to attach resp to associated dma_pkt during forward
            mem_pkt.append_delay(self.frontend_delay());
            mem_pkt.payload_mut().xbar_port_mut().push(token);
            self.mem
                .tx()
                .send_pkt(mem_pkt)
                .await
                .expect("Faulty Tx access");
        } else {
            // Network access
            let mut mem_req =
                MemBus::new(self.props.uid(), Command::Read, read_from.1, pattern, None);
            // Append register pkt_store id in xbar_port list
            // In will be used to attach resp to associated dma_pkt during forward
            mem_req.xbar_port_mut().push(token);

            let mut net_pkt =
                Network::new_wrapped(self.params.node_id.clone(), read_from.0, mem_req, None);

            // Append frontend delay
            net_pkt.append_delay(self.frontend_delay());
            self.net_outbound
                .tx()
                .send_pkt(net_pkt)
                .await
                .expect("Faulty Tx access");
        }
    }

    async fn forward_resp(&self, mut mem_pkt: Packet<MemBus>) {
        // Found associated entry
        let token = mem_pkt
            .payload_mut()
            .xbar_port_mut()
            .pop()
            .expect("Received unmatch MemBus response");

        let gen_pkt_kind = {
            let inner = self.inner.lock().unwrap();
            let store_entry = &inner.pkt_store[token];
            log!(|self| log::Category::Own, log::Verbosity::Trace  => token, store_entry);
            NDmaSlotKind::from(store_entry)
        };

        match gen_pkt_kind {
            NDmaSlotKind::None => panic!("Error with pkt_store management"),
            NDmaSlotKind::Dma => {
                // From local request
                let mem_pld = mem_pkt.payload_mut();
                log!(|self| log::Category::Own, log::Verbosity::Trace  => mem_pld.cmd(), => "Local");
                match mem_pld.cmd() {
                    Command::Read => {
                        // Clone useful info
                        let write_to = {
                            let inner = self.inner.lock().unwrap();
                            let dma_pld = &inner.pkt_store[token].as_dma_ref().unwrap().payload();
                            dma_pld.write_to().clone()
                        };

                        // Update mem_pkt
                        // -> Forge a Write Req from Read Resp
                        mem_pld.set_mode(Mode::Request);
                        mem_pld.set_cmd(Command::Write);
                        mem_pld.set_addr(write_to.1);
                        mem_pld.update_subrange();
                        // Store token for return path
                        mem_pld.xbar_port_mut().push(token);

                        let xfer_size = mem_pld.pattern().len();
                        mem_pkt.append_delay(self.forward_delay(xfer_size));

                        if write_to.0 == self.params.node_id {
                            // Send it back through outbound port
                            self.mem
                                .tx()
                                .send_pkt(mem_pkt)
                                .await
                                .expect("Faulty Tx access");
                        } else {
                            // Wrap it over network
                            // And force Response status since current Node is responding
                            //  to external one with red data
                            let mut net_pkt = Packet::inner_wrap(
                                self.params.node_id.clone(),
                                write_to.0,
                                mem_pkt,
                            );
                            net_pkt.payload_mut().set_mode(Mode::Response);

                            self.net_outbound.tx().fwd_pkt(net_pkt).await
                        }
                    }
                    Command::Write => {
                        // Dma xfer is finished
                        // Extract slot update and forward
                        let mut dma_pkt = self.remove_slot_at(token).unwrap_dma();
                        dma_pkt.payload_mut().set_mode(Mode::Response);

                        // Move upward
                        self.inbound.tx().fwd_pkt(*dma_pkt).await;
                    }
                    Command::AddrRange => panic!("Received AddrRange on NDma mem interface"),
                }
            }
            NDmaSlotKind::Net => {
                // External xfer is finished
                // Extract slot, build network packet back in response mode and forward
                let (from, to) = self.remove_slot_at(token).unwrap_net();
                let mut net_pkt = Packet::inner_wrap(from, to, mem_pkt);
                net_pkt.payload_mut().set_mode(Mode::Response);
                // Forward it over network
                self.net_inbound.tx().fwd_pkt(net_pkt).await
            }
        }
    }
    async fn forward_error(&self, mut mem_pkt: Packet<MemBus>, error: Arc<anyhow::Error>) {
        // Found associated entry
        let token = mem_pkt
            .payload_mut()
            .xbar_port_mut()
            .pop()
            .expect("Received unmatch MemBus response");

        // Error occurred report to dma initiator
        let gen_pkt = self.remove_slot_at(token);
        match gen_pkt {
            NDmaSlot::None => panic!("Error in pkt_store management"),
            NDmaSlot::Dma(mut dma_pkt) => {
                // Update mode
                dma_pkt
                    .payload_mut()
                    .set_mode(Mode::Error(DmaBusError::Inner(error)));
                // 2. Move upward
                self.inbound.tx().fwd_pkt(*dma_pkt).await;
            }
            NDmaSlot::Net { from, to } => {
                // Build back network packet and force mode as error
                let mut net_pkt = Packet::inner_wrap(from, to, mem_pkt);
                net_pkt
                    .payload_mut()
                    .set_mode(Mode::Error(NetworkError::Inner(error)));
                // Move over network
                self.net_inbound.tx().fwd_pkt(net_pkt).await;
            }
        }
    }
}

// #[cfg(test)]
// mod units_tests_dma {
//     use super::*;
//     use crate::mem;
//     use crate::test::Agent;
//     use ra2m_sim::prelude::{
//         port::ReqRespPort,
//         protocol::addr::{Addr, Pattern},
//         protocol::membus::Command,
//     };

//     use serial_test::serial;

//     #[tokio::test]
//     #[serial]
//     async fn test_dma() -> Result<(), anyhow::Error> {
//         Output::init("/tmp/ra2m/ra2m_cpn/integration_tests/test_dma");
//         // Create global simulation state and custom scheduler for hardware task
//         let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

//         // Instantiate and bind module
//         let mut root = module::Area::new(module::Properties::new(
//             "root".to_owned(),
//             Default::default(),
//         ));

//         // Xbar ===============================================================
//         root.insert_module(Arc::new(mem::XBar::new(
//             mem::XBarParams {
//                 inflight_req: 10,
//                 frontend_latency: types::Latency::Cycle(2.cycles()),
//                 forward_latency: types::Latency::Cycle(1.cycles()),
//                 bandwidth: 10.MiB_s(),
//                 inbound_cap: None,
//                 outbound_cap: None,
//             },
//             root.child_properties("xbar", Default::default()),
//         )));

//         // NpRam ==============================================================
//         root.insert_module(Arc::new(mem::NpRam::new(
//             mem::NpRamParams {
//                 ports: 2, // 1 for Dma, 1 for Agent
//                 size: 10.MB(),
//                 base_addr: Some(0),
//                 latency: types::Latency::Cycle(1.cycles()),
//                 bandwidth: 1.GB_s(),
//                 binfile: None,
//             },
//             root.child_properties("ram_src", Default::default()),
//         )));
//         root.inner_bind("xbar::outbound", "ram_src::resp_port")?;

//         root.insert_module(Arc::new(mem::NpRam::new(
//             mem::NpRamParams {
//                 ports: 2, // 1 for Dma, 1 for Agent
//                 size: 10.MB(),
//                 base_addr: Some(0x1000000),
//                 latency: types::Latency::Cycle(1.cycles()),
//                 bandwidth: 1.GB_s(),
//                 binfile: None,
//             },
//             root.child_properties("ram_dst", Default::default()),
//         )));
//         root.inner_bind("xbar::outbound", "ram_dst::resp_port")?;

//         // Dma ================================================================
//         root.insert_module(Arc::new(Dma::new(
//             DmaParams {
//                 inflight_req: 4,
//                 frontend_latency: types::Latency::Cycle(1.cycles()),
//                 forward_latency: types::Latency::Cycle(1.cycles()),
//                 bandwidth: 1.GB_s(),
//             },
//             root.child_properties("dma", Default::default()),
//         )));
//         root.inner_bind("dma::outbound", "xbar::inbound")?;

//         // Agent ==============================================================
//         let ram_agent_src = Arc::new(Agent::<ReqRespPort<MemBus>>::new(
//             root.child_properties("ram_agent_src", Default::default()),
//         ));
//         root.insert_module(ram_agent_src.clone());
//         root.inner_bind("ram_agent_src::port", "ram_src::resp_port")?;

//         let ram_agent_dst = Arc::new(Agent::<ReqRespPort<MemBus>>::new(
//             root.child_properties("ram_agent_dst", Default::default()),
//         ));
//         root.insert_module(ram_agent_dst.clone());
//         root.inner_bind("ram_agent_dst::port", "ram_dst::resp_port")?;

//         let dma_agent = Arc::new(Agent::<ReqRespPort<DmaBus<Addr>>>::new(
//             root.child_properties("dma_agent_dst", Default::default()),
//         ));
//         root.insert_module(dma_agent.clone());
//         root.inner_bind("dma_agent_dst::port", "dma::inbound")?;

//         // Init modules ======================================================
//         let root = Arc::new(root);
//         root.init();

//         // Test Body =========================================================
//         spawn_prc!(async {
//             event::Event::wait("AddrRangeMap_rdy").await;

//             // Gen pattern data and write in ram_src
//             let data = vec![0xfa; 2048];
//             let _ = ram_agent_src
//                 .clone()
//                 .b_req_resp_flush(MemBus::new(
//                     &0,
//                     Command::Write,
//                     Addr::Phys(0),
//                     Pattern::Simple(2048.Byte()),
//                     Some(data.as_slice()),
//                 ))
//                 .await
//                 .unwrap();

//             // Issued Dma xfer from src to dst
//             let _ = dma_agent
//                 .clone()
//                 .b_req_resp_flush(DmaBus::new(
//                     Addr::Phys(0),
//                     Addr::Phys(0x1000000),
//                     Pattern::Simple(2048.Byte()),
//                 ))
//                 .await
//                 .unwrap();
//             // Read back value in dest
//             let resp = ram_agent_dst
//                 .clone()
//                 .b_req_resp_flush(MemBus::new(
//                     &0,
//                     Command::Read,
//                     Addr::Phys(0),
//                     Pattern::Simple(2048.Byte()),
//                     None,
//                 ))
//                 .await
//                 .unwrap();
//             // check content
//             assert_eq!(
//                 data.as_slice(),
//                 resp.data().as_slice(),
//                 "Error: Src/Dst data mismatch"
//             );
//             event::Event::wait("EndOfSimulation").await;
//         });

//         // Start scheduler
//         sched.simulate(10.us().into()).await;
//         event::Event::triggered("EndOfSimulation", None);
//         Ok(())
//     }
// }
