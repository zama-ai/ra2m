//! SplitSwitch
//! Switch model that split the ReqRespPort in two indepedant channel Master/Slave
//! Req/Resp order are not preserved and could be shuffled by the switch
//!
//! NB: Since Req/Resp channel are split in bidir independent communication.
//! The Packet Mode is toggle to Response on forward path to prevent issue with
//! usual Req/Resp port check
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

#[derive(Module)]
pub struct SplitSwitch<T, P>
where
    T: 'static + std::cmp::Eq + std::hash::Hash + Clone + std::fmt::Debug + Send + Sync,
    for<'a> &'a T: Into<Traceable>,
    P: 'static + std::fmt::Debug + Send + Sync,
{
    params: super::SwitchParams,
    props: Arc<module::Properties>,
    #[port]
    port: port::PortVec<port::ReqRespPort<Network<T, P>>>,

    /// Header to port route
    port_map: RwLock<HashMap<T, usize>>,

    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[default_teardown]
impl<T, P> SplitSwitch<T, P>
where
    T: std::cmp::Eq + std::hash::Hash + Clone + std::fmt::Debug + Send + Sync,
    for<'a> &'a T: Into<Traceable>,
    P: std::fmt::Debug + Send + Sync,
{
    pub fn new(params: super::SwitchParams, props: module::Properties) -> Self {
        let port_cap = params.port_cap.unwrap_or(16);
        let props = Arc::new(props);
        Self {
            port: port::PortVec::new(
                port_cap,
                "port",
                props.clone(),
                Some(params.inflight_req),
                None,
            ),
            port_map: RwLock::new(HashMap::new()),
            prc: Mutex::new(Vec::new()),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        // Frontend layers -> 1/port to enable blocking wait
        for idx in 0..self.port.len() {
            let asc = self.clone();
            prc.push(spawn_prc!(Self::forward_layer(asc, idx)));
        }
    }

    /// Compute Switch delay on forward path
    fn switch_delay(&self, size: unit::Data) -> time::Tick {
        self.params
            .switch_latency
            .into_tick(self.properties().clock_domain())
            + time::Tick::from(size / self.params.bandwidth)
    }

    /// Wait on port and enqueue message in mpsc for process by forward layer
    async fn forward_layer(self: Arc<Self>, idx: usize) {
        let recv_port = &self.port[idx];
        loop {
            let mut pkt = recv_port.rx().wait_pkt().await;
            log!(|self| log::Category::Protocol, log::Verbosity::Trace  => pkt);

            let net = pkt.payload_mut();
            // Mode toggle to stay compliant with ReqResp port check
            net.set_mode(Mode::Response);
            let dst_port = self.get_port(net.to());

            // TODO correctly compute pkt_len
            let pkt_len = 0.Byte();

            match dst_port {
                Ok(dst) => {
                    log!(|self| log::Category::Own, log::Verbosity::Trace  => dst);
                    net.trace_mut().push(types::Handler::switch(
                        *self.properties().uid(),
                        types::SwitchPath::Forward(idx, dst),
                    ));

                    pkt.append_delay(self.switch_delay(pkt_len));
                    self.port[dst].tx().fwd_pkt(pkt).await;
                }
                Err(err) => {
                    net.set_mode(Mode::Error(err));
                    recv_port.tx().fwd_pkt(pkt).await;
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
