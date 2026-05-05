//! Switch
//! Simple Switch model
//! Listen to request and forward them to the targeted interface
use protocol::Mode;
use ra2m_sim::prelude::{
    protocol::network::{Network, NetworkError},
    *,
};

use std::sync::RwLock;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct SwitchParams {
    pub inflight_req: usize,

    // Latencies governing the time taken by a payload to go through
    // the switch.
    // Note that the switch itself does not handle the latency.
    // Instead the latency is annotated on the packet and left to the
    // neighbouring modules.
    pub switch_latency: types::Latency,

    /// Switch bandwidth
    pub bandwidth: unit::BW,

    /// Maximum capacity of vector port
    /// Option that default to 16 in unspecified
    pub port_cap: Option<usize>,
}

#[derive(Module)]
pub struct Switch<T, P>
where
    T: 'static + std::cmp::Eq + std::hash::Hash + Clone + std::fmt::Debug + Send + Sync,
    for<'a> &'a T: Into<Traceable>,
    P: 'static + std::fmt::Debug + Send + Sync,
{
    params: SwitchParams,
    props: Arc<module::Properties>,
    #[port]
    /// Received Network request
    ingress: port::PortVec<port::ReqRespPort<Network<T, P>>>,
    #[port]
    /// Forward Network request
    egress: port::PortVec<port::ReqRespPort<Network<T, P>>>,

    /// Header to port route
    port_map: RwLock<HashMap<T, usize>>,

    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[default_teardown]
impl<T, P> Switch<T, P>
where
    T: std::cmp::Eq + std::hash::Hash + Clone + std::fmt::Debug + Send + Sync,
    for<'a> &'a T: Into<Traceable>,
    P: std::fmt::Debug + Send + Sync,
{
    pub fn new(params: SwitchParams, props: module::Properties) -> Self {
        let port_cap = params.port_cap.unwrap_or(16);
        let props = Arc::new(props);
        Switch {
            ingress: port::PortVec::new(
                port_cap,
                "ingress",
                props.clone(),
                Some(params.inflight_req),
                None,
            ),
            egress: port::PortVec::new(port_cap, "egress", props.clone(), None, None),
            port_map: RwLock::new(HashMap::new()),
            prc: Mutex::new(Vec::new()),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        // Check port connection
        // Each interface must be connected to ingress/egress port
        let port_len = {
            let ingress_len = self.ingress.len();
            let egress_len = self.egress.len();
            assert_eq!(
                ingress_len, egress_len,
                "Current switch implementation require two port per interface"
            );
            ingress_len
        };

        let mut prc = self.prc.lock().unwrap();

        for idx in 0..port_len {
            // Forward layers
            let asc = self.clone();
            prc.push(spawn_prc!(Self::forward_layer(asc, idx)));
            // Backward layers
            let asc = self.clone();
            prc.push(spawn_prc!(Self::backward_layer(asc, idx)));
        }
    }

    /// Compute Switch delay on forward path
    fn switch_delay(&self, size: unit::Data) -> time::Tick {
        self.params
            .switch_latency
            .into_tick(self.properties().clock_domain())
            + time::Tick::from(size / self.params.bandwidth)
    }

    /// Forward request to targeted egress port
    /// Lock src/dst port and the associated layer for the transmission time
    async fn forward_layer(self: Arc<Self>, idx: usize) {
        let port = &self.ingress[idx];
        loop {
            let mut rx = port.rx().lock().await;
            let mut pkt = rx.recv().await;
            let req = pkt.payload_mut();
            log!(|self| log::Category::Protocol, log::Verbosity::Trace  => req);
            let dst_port = self.get_port(req.to());

            // TODO correctly compute pkt_len
            let pkt_len = 0.Byte();

            match dst_port {
                Ok(dst) => {
                    log!(|self| log::Category::Own, log::Verbosity::Trace  => dst);
                    req.trace_mut().push(types::Handler::switch(
                        *self.properties().uid(),
                        types::SwitchPath::Forward(idx, dst),
                    ));

                    pkt.append_delay(self.switch_delay(pkt_len));
                    self.egress[dst].tx().fwd_pkt(pkt).await;
                }
                Err(err) => {
                    req.set_mode(Mode::Error(err));
                    self.ingress[idx].tx().fwd_pkt(pkt).await;
                    continue;
                }
            }
        }
    }

    /// Backward dispatch of the response to associated inbound port
    /// Lock src/dst port and the associated layer for the transmission time
    async fn backward_layer(self: Arc<Self>, idx: usize) {
        let port = &self.egress[idx];
        loop {
            let mut rx = port.rx().lock().await;
            let mut pkt = rx.recv().await;
            let resp = pkt.payload_mut();
            log!(|self| log::Category::Protocol, log::Verbosity::Trace  => resp);
            let from_port = self.get_port(resp.from());

            // TODO correctly compute pkt_len
            let pkt_len = 0.Byte();
            match from_port {
                Ok(from) => {
                    log!(|self| log::Category::Own, log::Verbosity::Trace  => from);
                    resp.trace_mut().push(types::Handler::switch(
                        *self.properties().uid(),
                        types::SwitchPath::Backward(idx, from),
                    ));

                    pkt.append_delay(self.switch_delay(pkt_len));
                    self.ingress[from].tx().fwd_pkt(pkt).await;
                }
                Err(err) => {
                    resp.set_mode(Mode::Error(err));
                    self.egress[idx].tx().fwd_pkt(pkt).await;
                    continue;
                }
            }
        }
    }

    pub fn register_port(&self, trgt: &T, port: usize) -> Result<(), NetworkError> {
        let mut port_map = self.port_map.write().expect("Issue with RwLock");
        match port_map.try_insert(trgt.clone(), port) {
            Ok(_) => Ok(()),
            Err(_) => Err(NetworkError::AlreadyUsed),
        }
    }

    /// Read Getter for addr_map that handle the RwLock
    fn get_port(&self, descr: &T) -> Result<usize, NetworkError> {
        let port_map = self.port_map.read().unwrap();
        if let Some(port) = port_map.get(descr) {
            Ok(*port)
        } else {
            Err(NetworkError::Unreachable)
        }
    }
}
