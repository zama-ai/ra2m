//! IPC protocol
//!
//!
//! Define common Ipc types and associated commands
//! Ipc ffi rely an Req/Ack protocol on top of IPC
pub mod prelude;

use ipc_channel::ipc::{self, IpcOneShotServer, IpcReceiver, IpcSender};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Ipc error type
#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum IpcError {
    #[error("Invalid request @{addr}[{size_b} available range is [{range_lb},{range_ub}].")]
    AddrRange {
        addr: usize,
        size_b: usize,
        range_lb: usize,
        range_ub: usize,
    },
    #[error("Internal Ra2m error: {0}")]
    Ra2m(String),
    #[error("IPC Link error: {0}")]
    Ipc(String),
}

/// Ipc Request packet
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum IpcReq {
    Read {
        addr: u64,
        size_b: usize,
    },
    Write {
        addr: u64,
        data: ipc::IpcSharedMemory,
    },
}

/// Ipc acknowledgment packet
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum IpcAck {
    Read { data: ipc::IpcSharedMemory },
    Write(),
    Error(IpcError),
}

/// Master side of IPC channel. It issue request and received ack
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct IpcMaster {
    pub req: IpcSender<IpcReq>,
    pub ack: IpcReceiver<IpcAck>,
}

impl IpcMaster {
    /// Create IPC binding
    /// Use a named file to retrieved the OneShot IPC channel that enable to exchange
    /// typed ipc_channels
    pub fn new_bind_on(ipc_name: &str) -> Self {
        // Open file
        let mut rd_f = BufReader::new(
            OpenOptions::new()
                .create(false)
                .read(true)
                .open(ipc_name)
                .unwrap_or_else(|err| panic!("Error with {ipc_name}: {err}")),
        );
        // Read name of the targeted oneshot channel
        let oneshot_name = {
            let mut name = String::new();
            rd_f.read_line(&mut name).unwrap();
            name
        };
        // TODO use Ra2m internal tracing here
        // tracing::debug!("Will bind through {oneshot_name}");

        // Connet to the oneshot channel
        let bind_tx = IpcSender::connect(oneshot_name).unwrap();

        // Generate ipc channel and send slave side through oneshot
        let (master, slave) = ipc_channel();
        bind_tx.send(slave).unwrap();

        master
    }

    /// Send a request and wait for associated Ack
    pub fn b_req_ack(&mut self, req: IpcReq) -> Result<IpcAck, IpcError> {
        self.req
            .send(req)
            .map_err(|err| IpcError::Ipc(err.to_string()))?;
        self.ack
            .recv()
            .map_err(|err| IpcError::Ipc(err.to_string()))
    }
}

/// Slave side of the IPC channel. It received request and issue ack
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct IpcSlave {
    pub ack: IpcSender<IpcAck>,
    pub req: IpcReceiver<IpcReq>,
}

impl IpcSlave {
    /// Create IPC Oneshot server and wait for master request to establish the connection
    pub fn new_bind_on(ipc_name: &str) -> IpcSlave {
        // Create one shot channel
        let (oneshot_server, oneshot_name) = IpcOneShotServer::new().unwrap();

        // Register it into {ipc_name} file
        // Create folder if needed
        let path = Path::new(ipc_name);
        if let Some(dir_p) = path.parent() {
            std::fs::create_dir_all(dir_p).unwrap();
        }
        // Open file
        let mut wr_f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(ipc_name)
            .unwrap();
        write!(wr_f, "{oneshot_name}").unwrap();

        // TODO use Ra2m internal tracing here
        // tracing::info!("IpcSlave waiting on IPC `{oneshot_name}`");
        let (_, ipc_srv): (_, IpcSlave) = oneshot_server.accept().unwrap();

        ipc_srv
    }
}

pub fn ipc_channel() -> (IpcMaster, IpcSlave) {
    let (req_tx, req_rx) = ipc::channel().unwrap();
    let (ack_tx, ack_rx) = ipc::channel().unwrap();

    (
        IpcMaster {
            req: req_tx,
            ack: ack_rx,
        },
        IpcSlave {
            req: req_rx,
            ack: ack_tx,
        },
    )
}
