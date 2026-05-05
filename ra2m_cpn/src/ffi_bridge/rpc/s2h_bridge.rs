//! Sim To Host Bridge
//!
//! Implement RemotePort protocol for host <S-M> sim communication.
//! The RemortPort protocol is based on gRPC and enable host application to issues read/write
//! request in the simulated architecture.
//!
//! The gRPC rely on TCP or UnixDomain sockets

use ra2m_ffi::rpc::prelude::*;

use protocol::{
    addr::{Addr, Pattern, SubRangeAddr},
    membus::{Command, MemBus, MemBusError},
    Mode,
};
use ra2m_sim::prelude::*;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct S2hBridgeParams {
    /// s2h rpc socket Path
    pub rpc_path: String,
    /// Address range to register in Xbar if any
    pub addr_range: Option<(usize, unit::Data)>,
}

#[derive(Module)]
pub struct S2hBridge {
    params: S2hBridgeParams,
    props: Arc<module::Properties>,
    #[port]
    port: port::ReqRespPort<MemBus>,
    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl S2hBridge {
    pub fn new(params: S2hBridgeParams, props: module::Properties) -> Self {
        let props = Arc::new(props);
        S2hBridge {
            port: port::ReqRespPort::new("port", props.clone(), None, None),
            prc: Mutex::new(Vec::new()),
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();

        if let Some(ar) = self.params.addr_range {
            // Only support one contiguous addr_range
            let asc = self.clone();
            let addr_range = vec![Addr::Range(ar.0, ar.1)];
            prc.push(spawn_prc!(async {
                match protocol::membus::addr_range_register(asc.props.uid(), &asc.port, &addr_range)
                    .await
                {
                    Ok(_) => {}
                    Err(err) => {
                        log!(|asc| log::Category::Protocol, log::Verbosity::Warning => err );
                    }
                }
            }));
        }

        // Start fwd_over_rpc
        let asc = self.clone();
        prc.push(spawn_prc!(Self::fwd_over_rpc(asc)));
    }

    #[teardown]
    fn _teardown(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        while let Some(p) = prc.pop() {
            p.abort();
        }
    }

    /// Connect with gRPC endpoint to issue RemotePort RPC request
    async fn rpc_client(&self) -> Result<RemotePortClient<Channel>, anyhow::Error> {
        Ok(RemotePortClient::connect(self.params.rpc_path.clone()).await?)
    }

    /// Wait MemBus request on port and forward them on the RPC channel
    async fn fwd_over_rpc(self: Arc<Self>) {
        // Use to delayed rpc init to first access
        let mut rpc_client = None;
        loop {
            // Received request packet
            let mut rx = self.port.rx().lock().await;
            let mut pkt = rx.recv().await;
            drop(rx);

            // View as request and register in trace
            let req = pkt.payload_mut();
            req.trace_mut()
                .push(types::Handler::base(*self.properties().uid()));
            log!(|self| log::Category::Protocol, log::Verbosity::Trace => req);

            // Try to connect if not already done
            // TODO enhance init mechanisms, should be outside of the loop
            let rpc = if let Some(client_mut) = rpc_client.as_mut() {
                client_mut
            } else {
                rpc_client = match self.rpc_client().await {
                    Ok(rpc) => Some(rpc),
                    Err(err) => {
                        log!(|self| log::Category::Port, log::Verbosity::Error
                            => self.params.rpc_path, err
                            => "Unable to connect on RPC socket");
                        req.set_mode(Mode::Error(MemBusError::Ffi(format!("{err:?}"))));
                        self.port.tx().fwd_pkt(pkt).await;
                        continue;
                    }
                };
                rpc_client.as_mut().unwrap()
            };

            let addr = match req.subrange_addr() {
                SubRangeAddr::Phys(t) => *t,
                _ => {
                    req.set_mode(Mode::Error(MemBusError::SubRange(*req.subrange_addr())));
                    self.port.tx().fwd_pkt(pkt).await;
                    continue;
                }
            };

            match req.pattern() {
                Pattern::Simple(p) => {
                    match req.cmd() {
                        Command::Read => {
                            // Create Rp ReadRequest
                            let rd_req = tonic::Request::new(ReadRequest {
                                addr: addr as u64,
                                size: usize::from(p) as u64,
                            });
                            // TODO should unregister HW prc ?
                            match rpc.read(rd_req).await {
                                Ok(ack) => {
                                    req.data_mut()
                                        .extend_from_slice(ack.into_inner().data.as_slice());
                                }
                                Err(err) => {
                                    req.set_mode(Mode::Error(MemBusError::Ffi(format!("{err:?}"))));
                                    self.port.tx().fwd_pkt(pkt).await;
                                    continue;
                                }
                            }
                        }
                        Command::Write => {
                            let len = p.into();
                            let req_slice = req.data().as_slice();
                            assert_eq!(req_slice.len(), len);

                            // Create Rp WriteRequest
                            let mut data = Vec::with_capacity(len);
                            data.extend_from_slice(req_slice);
                            let wr_req = tonic::Request::new(WriteRequest {
                                addr: addr as u64,
                                data,
                            });
                            // TODO should unregister HW prc ?
                            match rpc.write(wr_req).await {
                                Ok(_ack) => {}
                                Err(err) => {
                                    req.set_mode(Mode::Error(MemBusError::Ffi(format!("{err:?}"))));
                                    self.port.tx().fwd_pkt(pkt).await;
                                    continue;
                                }
                            }
                        }
                        _ => {
                            req.set_mode(Mode::Error(MemBusError::Cmd(*req.cmd())));
                            self.port.tx().fwd_pkt(pkt).await;
                            continue;
                        }
                    };
                }
                _ => {
                    req.set_mode(Mode::Error(MemBusError::Pattern(*req.pattern())));
                    self.port.tx().fwd_pkt(pkt).await;
                    continue;
                }
            }

            // Success path
            req.set_mode(Mode::Response);
            self.port.tx().fwd_pkt(pkt).await;
        }
    }
}
