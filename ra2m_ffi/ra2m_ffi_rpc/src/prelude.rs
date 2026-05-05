//! Prelude for Rpc ffi
pub use super::RemotePortEndpoint;

pub use remote_port::remote_port_client::RemotePortClient;
pub use remote_port::remote_port_server::{RemotePort, RemotePortServer};
pub use remote_port::{ReadAck, ReadRequest, WriteAck, WriteRequest};
pub use std::any::Any;

pub mod remote_port {
    tonic::include_proto!("remote_port");
}

pub use tonic;
pub use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};
