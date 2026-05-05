//! Utility structure for Rpc RemotePort Client
//!
//! Provide structure to handle Rpc endpoint and abstract RemotePort Request/Response
//! Also provide a set of macro to ease update of structure subpart only over rpc channel
//!
use remote_port::remote_port_client::RemotePortClient;
use remote_port::{ReadRequest, WriteRequest};
use std::error::Error;
use tonic::transport::Channel;

pub mod remote_port {
    tonic::include_proto!("remote_port");
}

/// Endpoint implementation
/// Used to issue Read/Write request to associated server
pub struct RemotePortEndpoint {
    rp_ep: RemotePortClient<Channel>,
}

impl RemotePortEndpoint {
    pub async fn new(rpc_path: &str) -> Result<Self, Box<dyn Error>> {
        let rp_ep = {
            let rpc_path = rpc_path.to_string();
            RemotePortClient::connect(rpc_path).await?
        };

        Ok(Self { rp_ep })
    }

    pub async fn rp_read(&mut self, addr: u64, size: u64) -> Result<Vec<u8>, Box<dyn Error>> {
        let req = tonic::Request::new(ReadRequest { addr, size });

        let ack = self.rp_ep.read(req).await?;
        Ok(ack.into_inner().data)
    }

    pub async fn rp_write(&mut self, addr: u64, data: Vec<u8>) -> Result<(), Box<dyn Error>> {
        let req = tonic::Request::new(WriteRequest { addr, data });
        let _ack = self.rp_ep.write(req).await?;
        // Ok(ack.into_inner())
        Ok(())
    }
}

impl std::ops::Deref for RemotePortEndpoint {
    type Target = RemotePortClient<Channel>;

    fn deref(&self) -> &Self::Target {
        &self.rp_ep
    }
}
impl std::ops::DerefMut for RemotePortEndpoint {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.rp_ep
    }
}
