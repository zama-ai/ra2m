//! XBar CrossBar bus
//! CrossBar bus model with multiple layer vectorized ReqRespPort based on MemoryBus protocol.
//! Listen to request and forward them to the targeted address range
use protocol::{
    addr::{Addr, AddrRangeMap, SubRangeAddr},
    membus::{Command, MemBus, MemBusError},
    Mode,
};
use ra2m_sim::prelude::*;

use std::sync::{Arc, Mutex};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Debug, Clone)]
pub struct XBarParams {
    pub inflight_req: usize,

    // Latencies governing the time taken for the various paths a
    // packet has through the crossbar. Note that the crossbar itself
    // does not handle the latency. Instead the latency is annotated on
    // the packet and left to the neighbouring modules.
    //
    // A request incurs the frontend latency and forward latency.
    // A response incurs the response latency.
    /// Frontend latency encompasses arbitration and deciding what to do when a
    /// request arrives.
    pub frontend_latency: types::Latency,
    /// Forward latency is the latency involved once a decision is made to
    /// forward the request or the response.
    pub forward_latency: types::Latency,

    /// XBar bus width
    /// Number of bits handled every cycles
    /// ATM: use bandwidth instead of bit-width per cycles because module
    /// don't implement ClockDomain yet
    // pub width: unit::Data,
    pub bandwidth: unit::BW,

    /// Maximum capacity of inbound vector port
    /// Option that default to 16 in unspecified
    pub inbound_cap: Option<usize>,

    /// Maximum capacity of outbound vector port
    /// Option that default to 16 in unspecified
    pub outbound_cap: Option<usize>,
}

/// AddrRange representation
/// Encode Addr offset and associated size for an address range
/// NB: Offset could be expressed in Physical or Virtual address space
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddrRange {
    offset: Addr,
    range: usize,
}

#[derive(Module)]
pub struct XBar {
    params: XBarParams,
    props: Arc<module::Properties>,
    /// inbound: Received request and forward to associated addr_range
    /// Lock all inbound together to poll over them => vector instance shared mutex
    #[port]
    inbound: port::PortVec<port::ReqRespPort<MemBus>>,

    /// outbound: Forward request to associated addr_range
    /// Lock them one by one => each entry have its own mutex
    #[port]
    outbound: port::PortVec<port::ReqRespPort<MemBus>>,

    /// Addr_range route
    /// Map of AddrRange::outbound_id
    addr_map: RwLock<AddrRangeMap>,

    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[default_teardown]
impl XBar {
    pub fn new(params: XBarParams, props: module::Properties) -> Self {
        let inbound_cap = params.inbound_cap.unwrap_or(16);
        let outbound_cap = params.outbound_cap.unwrap_or(16);
        let props = Arc::new(props);
        XBar {
            inbound: port::PortVec::new(inbound_cap, "inbound", props.clone(), None, None),
            outbound: port::PortVec::new(
                outbound_cap,
                "outbound",
                props.clone(),
                Some(params.inflight_req),
                None,
            ),
            addr_map: RwLock::new(AddrRangeMap::new()),
            prc: Mutex::new(Vec::new()),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        // get size of inbound and outbound port
        let inbound_len = self.inbound.len();
        let outbound_len = self.outbound.len();

        let mut prc = self.prc.lock().unwrap();
        // Register oneshot process that will retrieved addr range from outbound
        let asc = self.clone();
        prc.push(spawn_prc!(XBar::addr_range_lookup(asc)));

        // Forward layers
        for idx in 0..inbound_len {
            let asc = self.clone();
            prc.push(spawn_prc!(XBar::forward_layer(asc, idx)));
        }
        // Backward layers
        for idx in 0..outbound_len {
            let asc = self.clone();
            prc.push(spawn_prc!(XBar::backward_layer(asc, idx)));
        }
    }

    /// Read Getter for addr_map that handle the RwLock
    fn addr_map(&self) -> RwLockReadGuard<'_, AddrRangeMap> {
        self.addr_map.read().unwrap()
    }

    /// Write Getter for addr_map that handle the RwLock
    fn addr_map_mut(&self) -> RwLockWriteGuard<'_, AddrRangeMap> {
        self.addr_map.write().unwrap()
    }

    /// Compute XBar delay on forward path
    fn forward_delay(&self, size: unit::Data) -> time::Tick {
        self.params
            .frontend_latency
            .into_tick(self.properties().clock_domain())
            + self
                .params
                .forward_latency
                .into_tick(self.properties().clock_domain())
            + time::Tick::from(size / self.params.bandwidth)
    }

    /// Compute XBar delay on backward path
    fn backward_delay(&self, size: unit::Data) -> time::Tick {
        self.params
            .forward_latency
            .into_tick(self.properties().clock_domain())
            + time::Tick::from(size / self.params.bandwidth)
    }

    /// OneShot prc that wait for all addr_range request from outbound port
    async fn addr_range_lookup(self: Arc<Self>) {
        for (i, p) in self.outbound.as_slice().iter().enumerate() {
            let mut rx = p.rx().0.lock().await;
            loop {
                let pkt = rx.recv().await;
                let req = pkt.payload();
                log!(|self| log::Category::Protocol, log::Verbosity::Trace  => req);
                match req.cmd() {
                    Command::AddrRange => {
                        match req.addr() {
                            Addr::Range(addr, size) => {
                                log!(|self| log::Category::Own, log::Verbosity::Debug
                                => addr, size => "AddrRange insertion request");
                                self.addr_map_mut()
                                    .insert_range(*addr, *size, i)
                                    .expect("AddrMap insertion failed");
                            }
                            _ => {
                                // End of AddrRange setup for this port
                                break;
                            }
                        }
                    }
                    _ => {
                        log!(|self| log::Category::Own, log::Verbosity::Error
                            => => "MemoryBus over ReqRespPort must start by AddrRange sequence");
                        panic! {"MemoryBus over ReqRespPort must start by AddrRange sequence"}
                    }
                };
            }
            log!(|self| log::Category::Own, log::Verbosity::Debug
                => i => "Route lookup done for outbound[i]");
        }
        log!(|self| log::Category::Own, log::Verbosity::Debug
            => self.addr_map => "Finish route lookup");

        // Notify Forward and backward layers
        event::Event::triggered("AddrRangeMap_rdy", Some(10));
    }

    /// Forward dispatch of the request to the targeted outbound port
    async fn forward_layer(self: Arc<Self>, idx: usize) {
        // Wait on addr_range/ ar_route ready
        event::Event::wait("AddrRangeMap_rdy").await;

        let port = &self.inbound[idx];
        loop {
            let mut rx = port.rx().lock().await;
            let mut pkt = rx.recv().await;
            let req = pkt.payload_mut();
            log!(|self| log::Category::Protocol, log::Verbosity::Trace  => req);
            let addr_ofst = match req.subrange_addr() {
                SubRangeAddr::Phys(addr) => *addr,
                _ => {
                    req.set_mode(Mode::Error(MemBusError::SubRange(*req.subrange_addr())));
                    self.inbound[idx].tx().fwd_pkt(pkt).await;
                    continue;
                }
            };
            let range = req.pattern().range();
            let acc_len = match req.cmd() {
                Command::Write => req.pattern().len(),
                Command::Read => 0.Byte(),
                _ => {
                    req.set_mode(Mode::Error(MemBusError::Cmd(*req.cmd())));
                    self.inbound[idx].tx().fwd_pkt(pkt).await;
                    continue;
                }
            };

            // Update xbar port stack for backward path
            req.xbar_port_mut().push(idx);

            let dst_port = self.addr_map().find_port(addr_ofst, range);
            match dst_port {
                Ok((ar_ofst, dst)) => {
                    // Update addr_align to match with sub-range properties
                    let subrange_updt = match req.subrange_addr() {
                        SubRangeAddr::Phys(addr) => SubRangeAddr::Phys(*addr - ar_ofst),
                        _ => {
                            req.set_mode(Mode::Error(MemBusError::SubRange(*req.subrange_addr())));
                            self.inbound[idx].tx().fwd_pkt(pkt).await;
                            continue;
                        }
                    };
                    log!(|self| log::Category::Own, log::Verbosity::Trace  => dst);
                    req.set_subrange_addr(subrange_updt);
                    req.trace_mut().push(types::Handler::switch(
                        *self.properties().uid(),
                        types::SwitchPath::Forward(idx, dst),
                    ));

                    // Lock the port and the associated layer for the transmission time
                    let fwd_ep = &self.outbound[dst].tx();
                    pkt.append_delay(self.forward_delay(acc_len));
                    fwd_ep.fwd_pkt(pkt).await;
                }
                Err(err) => {
                    req.set_mode(Mode::Error(MemBusError::AddrRange(err)));
                    self.inbound[idx].tx().fwd_pkt(pkt).await;
                    continue;
                }
            }
        }
    }

    /// Backward dispatch of the response to associated inbound port
    async fn backward_layer(self: Arc<Self>, idx: usize) {
        // Wait on addr_range/ ar_route ready
        event::Event::wait("AddrRangeMap_rdy").await;

        let port = &self.outbound[idx];
        loop {
            let mut rx = port.rx().lock().await;
            let mut pkt = rx.recv().await;
            let req = pkt.payload_mut();
            log!(|self| log::Category::Protocol, log::Verbosity::Trace  => req);
            if let Some(from_port) = req.xbar_port_mut().pop() {
                log!(|self| log::Category::Own, log::Verbosity::Trace  => from_port);
                let fwd_ep = &self.inbound[from_port].tx();
                let acc_len = match req.cmd() {
                    Command::Read => req.pattern().len(),
                    Command::Write => 0.Byte(),
                    _ => {
                        panic!("Unsupported command {:?}", req.cmd());
                    }
                };
                req.trace_mut().push(types::Handler::switch(
                    *self.properties().uid(),
                    types::SwitchPath::Backward(from_port, idx),
                ));

                // Lock the port and the associated layer for the transmission time
                pkt.append_delay(self.backward_delay(acc_len));
                fwd_ep.fwd_pkt(pkt).await;
            } else {
                panic!("{}: {}:backward_layer[{idx}] Incorrect management of Xbar queue, it must contain a value {req:?}", cur_tick(), self.properties().path());
            }
        }
    }
}
