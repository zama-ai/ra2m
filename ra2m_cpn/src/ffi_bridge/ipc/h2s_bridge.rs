//! Host to Sim Bridge
//!
//! Implement Req/Ack protocol for host <M-S> sim communication.
//! It's based on IPC and enable host application to issues read/write
//! request in the simulated architecture.

use protocol::{
    addr::{Addr, Pattern},
    membus::{Command, MemBus},
};
use ra2m_ffi::ipc::prelude::*;
use ra2m_sim::prelude::*;

use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct H2sBridgeParams {
    /// h2s ipc socket Path
    pub ipc_path: String,
    /// Host only have a reduced view of the simulated memory
    pub addr_range: (usize, unit::Data),
    /// Number of outstanding request supported
    pub inflight_req: usize,
    /// Simulation time between 2 polling of Ipc channel
    pub polling_rate: unit::Time,
    /// Spawn an prc to keep alive the simulation
    pub keep_alive: Option<unit::Time>,
}

#[derive(Module)]
pub struct H2sBridge {
    params: H2sBridgeParams,
    props: Arc<module::Properties>,
    #[port]
    port: port::ReqRespPort<MemBus>,
    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    ipc: Mutex<Option<IpcSlave>>,
}

impl H2sBridge {
    pub fn new(params: H2sBridgeParams, props: module::Properties) -> Self {
        let props = Arc::new(props);

        Self {
            port: port::ReqRespPort::new("port", props.clone(), Some(params.inflight_req), None),
            prc: Mutex::new(Vec::new()),
            params,
            props,
            ipc: Mutex::new(None),
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        // Start ipc_polling routine
        let asc = self.clone();
        prc.push(spawn_prc!(Self::ipc_polling(asc)));

        // Start keep-alive process if required
        if let Some(delay) = self.params.keep_alive {
            let asc = self.clone();
            prc.push(spawn_prc!(Self::keep_alive_process(asc, delay)));
        }
    }

    #[teardown]
    fn _teardown(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        while let Some(p) = prc.pop() {
            p.abort();
        }
        // Remove ipc bind file
        std::fs::remove_file(Path::new(&self.params.ipc_path)).unwrap();
    }

    fn check_range(&self, req: &IpcReq) -> Result<(), IpcError> {
        let bridge_lb = self.params.addr_range.0;
        let bridge_ub = bridge_lb + usize::from(self.params.addr_range.1);
        let (req_lb, size_b) = match req {
            IpcReq::Read { addr, size_b } => (*addr as usize, *size_b),
            IpcReq::Write { addr, data } => (*addr as usize, data.len()),
        };

        let req_ub = req_lb + size_b;
        if (bridge_lb <= req_lb) && (req_ub <= bridge_ub) {
            Ok(())
        } else {
            Err(IpcError::AddrRange {
                addr: req_lb,
                size_b,
                range_lb: bridge_lb,
                range_ub: bridge_ub,
            })
        }
    }

    /// Wait for ipc binding
    /// WARN: this task is blocking, thus it triggered a prc_unregister rely on tokio spawn_blocking and then register back the task
    async fn ipc_bind(ipc_path: String) -> IpcSlave {
        // Unregister from Hw scheduler to prevent deadlock
        scheduler::SchedulerAPI::unregister_prc();
        let ipc = tokio::task::spawn_blocking(move || IpcSlave::new_bind_on(&ipc_path))
            .await
            .unwrap();
        // Register back in Hw scheduler
        scheduler::SchedulerAPI::register_new_prc();
        ipc
    }

    /// Periodically poll IpcChannel
    async fn ipc_polling(self: Arc<Self>) {
        loop {
            // Check for bind ipc
            let bind = {
                let ipc_lock = self.ipc.lock().expect("Error in H2sBridge ipc locking");
                ipc_lock.is_some()
            };
            if !bind {
                // start bind procedure
                let ipc = Self::ipc_bind(self.params.ipc_path.clone()).await;
                // update ipc field
                let mut ipc_lock = self.ipc.lock().expect("Error in H2sBridge ipc locking");
                *ipc_lock = Some(ipc);
            }

            // Check for available message
            let pkt = {
                let mut ipc_lock = self.ipc.lock().expect("Error in H2sBridge ipc locking");
                let ipc = ipc_lock.as_ref().expect("Ipc binding issue");
                match ipc.req.try_recv() {
                    Ok(pkt) => {
                        log!(|self| log::Category::Own, log::Verbosity::Trace => pkt =>"IpcSlave Recv");
                        Some(pkt)
                    }
                    Err(err) => match &err {
                        ipc_channel::ipc::TryRecvError::IpcError(kind) => match kind {
                            ipc_channel::ipc::IpcError::Disconnected => {
                                // Reset ipc channel
                                *ipc_lock = None;
                                None
                            }
                            _ => panic!("Encounter Ipc error {err:?}"),
                        },
                        ipc_channel::ipc::TryRecvError::Empty => None,
                    },
                }
            };

            if let Some(req) = pkt {
                let ack = match self.check_range(&req) {
                    Ok(_) => match req {
                        IpcReq::Read { addr, size_b } => {
                            // Translate Ipc request in Membus Request
                            let resp = self
                                .port
                                .b_req_resp(MemBus::new_wrapped(
                                    self.props.uid(),
                                    Command::Read,
                                    Addr::Phys(addr as usize),
                                    Pattern::Simple(size_b.Byte()),
                                    None,
                                    None,
                                ))
                                .await;

                            // Translate MemBus response in RemotePort Ack
                            match resp {
                                Ok(pkt) => {
                                    // View packet as payload and copy data
                                    let pld = pkt.payload();
                                    IpcAck::Read {
                                        data: ipc_channel::ipc::IpcSharedMemory::from_bytes(
                                            pld.data().as_slice(),
                                        ),
                                    }
                                }
                                Err(err) => {
                                    log!(|self| log::Category::Own, log::Verbosity::Error => err);
                                    IpcAck::Error(IpcError::Ra2m(err.to_string()))
                                }
                            }
                        }
                        IpcReq::Write { addr, data } => {
                            // Translate Ipc request in Membus Request
                            let resp = self
                                .port
                                .b_req_resp(MemBus::new_wrapped(
                                    self.props.uid(),
                                    Command::Write,
                                    Addr::Phys(addr as usize),
                                    Pattern::Simple(data.len().Byte()),
                                    Some(&data),
                                    None,
                                ))
                                .await;

                            // Translate MemBus response in RemotePort Ack
                            match resp {
                                Ok(_pkt) => IpcAck::Write(),
                                Err(err) => {
                                    log!(|self| log::Category::Own, log::Verbosity::Error => err);
                                    IpcAck::Error(IpcError::Ra2m(err.to_string()))
                                }
                            }
                        }
                    },

                    Err(err) => {
                        log!(|self| log::Category::Own, log::Verbosity::Error => err);
                        IpcAck::Error(err)
                    }
                };

                let ipc_lock = self.ipc.lock().expect("Error in H2sBridge ipc locking");
                let ipc = ipc_lock.as_ref().expect("Ipc binding issue");
                let _ = ipc.ack.send(ack);
            } else {
                delay::Delay::wait_for(self.params.polling_rate.into()).await;
            }
        }
    }

    /// Periodically schedule a wait event
    /// Force the simulation to keep going even if no other simulation events are available.
    /// Here to keep simulation alive until new work come from the host bridge interface (untimed)
    /// Also prevent simulation to run faster than real time
    async fn keep_alive_process(self: Arc<Self>, delay: unit::Time) {
        loop {
            let rt_start = std::time::Instant::now();
            delay::Delay::wait_for(delay.into()).await;
            let rt_duration = rt_start.elapsed();

            let req_delay_ns = delay.into_fs() / 10_usize.pow(6);
            let rt_duration_ns: usize = rt_duration.as_nanos().try_into().unwrap();
            if req_delay_ns > rt_duration_ns {
                // add extra delay to slow down simulation
                std::thread::sleep(std::time::Duration::from_nanos(
                    (req_delay_ns - rt_duration_ns) as u64,
                ));
            }
        }
    }
}
