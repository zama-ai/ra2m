//! Sim To Host Bridge
//!
//! Implement Req/Ack protocol for host <S-M> sim communication.
//! It's based on IPC and enable simulated architecture to issues read/write
//! request in the host application.

use protocol::{
    addr::{Addr, Pattern, SubRangeAddr},
    membus::{Command, MemBus, MemBusError},
    Mode,
};
use ra2m_ffi::ipc::prelude::*;
use ra2m_sim::prelude::*;

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct S2hBridgeParams {
    /// s2h ipc socket Path
    pub ipc_path: String,
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
        Self {
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

        // Start fwd_over_ipc
        let asc = self.clone();
        prc.push(spawn_prc!(Self::fwd_over_ipc(asc)));
    }

    #[teardown]
    fn _teardown(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        while let Some(p) = prc.pop() {
            p.abort();
        }
    }

    /// Wait MemBus request on port and forward them on the IPC channel
    async fn fwd_over_ipc(self: Arc<Self>) {
        // Use to delayed ipc init to first access
        let mut ipc_master = None;
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
            let ipc = if let Some(master_mut) = ipc_master.as_mut() {
                master_mut
            } else {
                ipc_master = Some(IpcMaster::new_bind_on(&self.params.ipc_path));
                ipc_master.as_mut().unwrap()
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
                            // Create Ipc ReadRequest and send it over Ipc
                            let rd_req = IpcReq::Read {
                                addr: addr as u64,
                                size_b: usize::from(p),
                            };
                            let _ = ipc.req.send(rd_req);
                            log!(|self| log::Category::Own, log::Verbosity::Trace => "IpcMaster forward {rd_req:x?}");

                            // Wait for ack
                            // TODO should unregister HW prc during Ipc roundtrip ?
                            match ipc.ack.recv() {
                                Ok(ack) => {
                                    log!(|self| log::Category::Own, log::Verbosity::Trace => "IpcMaster received Ack {ack:x?}");
                                    match ack {
                                        IpcAck::Read { data } => {
                                            // Populate initial request with received data
                                            req.data_mut().extend_from_slice(&data);
                                        }
                                        _ => {
                                            req.set_mode(Mode::Error(MemBusError::Ffi(format!(
                                                "Received Ack mismatch with sent request {ack:x?}"
                                            ))));
                                            self.port.tx().fwd_pkt(pkt).await;
                                            continue;
                                        }
                                    }
                                }
                                Err(err) => {
                                    req.set_mode(Mode::Error(MemBusError::Ffi(format!("{err:?}"))));
                                    self.port.tx().fwd_pkt(pkt).await;
                                    continue;
                                }
                            }
                        }
                        Command::Write => {
                            let len: usize = p.into();
                            let req_slice = req.data().as_slice();
                            assert_eq!(req_slice.len(), len);

                            // Create Ipc WriteRequest and send it over Ipc
                            let wr_req = IpcReq::Write {
                                addr: addr as u64,
                                data: ipc_channel::ipc::IpcSharedMemory::from_bytes(
                                    req.data().as_slice(),
                                ),
                            };
                            let _ = ipc.req.send(wr_req);
                            log!(|self| log::Category::Own, log::Verbosity::Trace => "IpcMaster forward {wr_req:x?}");

                            // Wait for ack
                            // TODO should unregister HW prc during Ipc roundtrip ?
                            match ipc.ack.recv() {
                                Ok(ack) => {
                                    log!(|self| log::Category::Own, log::Verbosity::Trace => "IpcMaster received Ack {ack:x?}");
                                    match ack {
                                        IpcAck::Write() => {}
                                        _ => panic!(
                                            "Received Ack mismatch with sent request {ack:x?}"
                                        ),
                                    }
                                }
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
