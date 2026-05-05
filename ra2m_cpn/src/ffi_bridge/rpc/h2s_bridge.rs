//! Host to Sim Bridge
//!
//! Implement RemotePort protocol for host <M-S> sim communication.
//! The RemortPort protocol is based on gRPC and enable host application to issues read/write
//! request in the simulated architecture.
//!
//! The gRPC rely on TCP or UnixDomain sockets

use ra2m_ffi::rpc::prelude::*;

use protocol::{
    addr::{Addr, Pattern},
    membus::{Command, MemBus},
};
use ra2m_sim::prelude::*;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct H2sBridgeParams {
    /// h2s rpc socket Path
    pub rpc_path: String,
    /// Host only have a reduced view of the simulated memory
    pub addr_range: (usize, unit::Data),
    /// Number of outstanding request supported
    pub inflight_req: usize,
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
}
impl H2sBridge {
    #[init]
    fn _init(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        // Start rpc server
        let asc = self.clone();
        prc.push(H2sBridgeWrapped::rpc_server(asc.into()));

        // Start keep-alive process if required
        if let Some(delay) = self.params.keep_alive {
            let asc = self.clone();
            prc.push(spawn_prc!(H2sBridgeWrapped::keep_alive_process(
                asc.into(),
                delay
            )));
        }
    }

    #[teardown]
    fn _teardown(self: Arc<Self>) {
        let mut prc = self.prc.lock().unwrap();
        while let Some(p) = prc.pop() {
            p.abort();
        }
    }
}

impl H2sBridge {
    pub fn new(params: H2sBridgeParams, props: module::Properties) -> Self {
        let props = Arc::new(props);
        H2sBridge {
            port: port::ReqRespPort::new("port", props.clone(), Some(params.inflight_req), None),
            prc: Mutex::new(Vec::new()),
            params,
            props,
        }
    }

    #[allow(clippy::result_large_err)]
    fn check_read_range(&self, req: &ReadRequest) -> Result<(), Status> {
        let bridge_lb = self.params.addr_range.0;
        let bridge_ub = bridge_lb + usize::from(self.params.addr_range.1);
        let rd_lb = req.addr as usize;
        let rd_ub = rd_lb + req.size as usize;

        if (bridge_lb <= rd_lb) && (rd_ub <= bridge_ub) {
            Ok(())
        } else {
            let err_msg = format!(
                "Invalid addr range for {req:?} expect access within [{bridge_lb}:{bridge_ub}]"
            );
            Err(Status::new(tonic::Code::InvalidArgument, err_msg))
        }
    }

    #[allow(clippy::result_large_err)]
    fn check_write_range(&self, req: &WriteRequest) -> Result<(), Status> {
        let bridge_lb = self.params.addr_range.0;
        let bridge_ub = bridge_lb + usize::from(self.params.addr_range.1);
        let wr_lb = req.addr as usize;
        let wr_ub = wr_lb + req.data.len();

        if (bridge_lb <= wr_lb) && (wr_ub <= bridge_ub) {
            Ok(())
        } else {
            let err_msg = format!(
                "Invalid addr range for {req:?} expect access within [{bridge_lb}:{bridge_ub}]"
            );
            Err(Status::new(tonic::Code::InvalidArgument, err_msg))
        }
    }
}

pub struct H2sBridgeWrapped(Arc<H2sBridge>);
impl std::ops::Deref for H2sBridgeWrapped {
    type Target = Arc<H2sBridge>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Arc<H2sBridge>> for H2sBridgeWrapped {
    fn from(value: Arc<H2sBridge>) -> Self {
        Self(value)
    }
}

impl H2sBridgeWrapped {
    /// Spawn gRPC server to handle RemotePort RPC request
    fn rpc_server(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let addr = self.params.rpc_path.parse().unwrap();
            Server::builder()
                .add_service(RemotePortServer::new(self))
                .serve(addr)
                .await
                .unwrap();
        })
    }

    /// Periodically schedule a wait event
    /// Force the simulation to keep going even if no other simulation events are available.
    /// Here to keep simulation alive until new work come from the host bridge interface (untimed)
    /// Also prevent simulation to run faster than real time
    async fn keep_alive_process(self, delay: unit::Time) {
        loop {
            let rt_start = Instant::now();
            delay::Delay::wait_for(delay.into()).await;
            let rt_duration = rt_start.elapsed();

            let req_delay_ns = delay.into_fs() / 10_usize.pow(6);
            let rt_duration_ns: usize = rt_duration.as_nanos().try_into().unwrap();
            if req_delay_ns > rt_duration_ns {
                // add extra delay to slow down simulation
                std::thread::sleep(Duration::from_nanos((req_delay_ns - rt_duration_ns) as u64));
            }
        }
    }
}
#[tonic::async_trait]
impl RemotePort for H2sBridgeWrapped {
    #[allow(clippy::result_large_err)]
    async fn read(&self, request: Request<ReadRequest>) -> Result<Response<ReadAck>, Status> {
        let request = request.into_inner();

        // Check that request belong in Bridge range
        self.check_read_range(&request)?;

        // Translate RemotePort request in Membus Request
        // Also register it temporary in the Hw scheduler
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        let resp = self
            .port
            .b_req_resp(MemBus::new_wrapped(
                self.props.uid(),
                Command::Read,
                Addr::Phys(request.addr as usize),
                Pattern::Simple(request.size.Byte()),
                None,
                None,
            ))
            .await;
        scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit

        // Translate MemBus response in RemotePort Ack
        match resp {
            Ok(pkt) => {
                // View packet as payload and copy data
                let pld = pkt.payload();
                let mut data = Vec::new();
                data.extend_from_slice(pld.data().as_slice());

                let ack = remote_port::ReadAck { data };
                Ok(Response::new(ack))
            }
            Err(err) => {
                let err_msg = format!("{err:?}");
                Err(Status::new(tonic::Code::Internal, err_msg))
            }
        }
    }

    #[allow(clippy::result_large_err)]
    async fn write(&self, request: Request<WriteRequest>) -> Result<Response<WriteAck>, Status> {
        let request = request.into_inner();

        // Check that request belong in Bridge range
        self.check_write_range(&request)?;

        // Translate RemotePort request in Membus Request
        // Also register it temporary in the Hw scheduler
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        let resp = self
            .port
            .b_req_resp(MemBus::new_wrapped(
                self.props.uid(),
                Command::Write,
                Addr::Phys(request.addr as usize),
                Pattern::Simple(request.data.len().Byte()),
                Some(request.data.as_slice()),
                None,
            ))
            .await;
        scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit

        // Translate MemBus response in RemotePort Ack
        match resp {
            Ok(_pkt) => {
                let ack = remote_port::WriteAck {};
                Ok(Response::new(ack))
            }
            Err(err) => {
                let err_msg = format!("{err:?}");
                Err(Status::new(tonic::Code::Internal, err_msg))
            }
        }
    }
}
