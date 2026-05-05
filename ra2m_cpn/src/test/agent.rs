//! Driver agent with generic port
//!
//! For ReqRespPort it expose custom method for round-trip communication

use ra2m_sim::prelude::*;
use std::sync::Arc;

#[derive(Module)]
pub struct Agent<P>
where
    P: 'static + port::Port + port::PortNew + Send + Sync,
{
    props: Arc<module::Properties>,
    #[port]
    port: P,
}

#[default_init]
#[default_teardown]
impl<P> Agent<P>
where
    P: 'static + port::Port + port::PortNew + Send + Sync,
{
    pub fn new(props: module::Properties) -> Self {
        let props = Arc::new(props);
        Self {
            port: port::PortNew::new("port", props.clone(), Some(1), None),
            props,
        }
    }
}

impl<T> Agent<port::ReqRespPort<T>>
where
    T: 'static + TxStatus + RxStatus + Send + std::fmt::Debug + Trace + serde::Serialize,
{
    pub async fn b_req_resp(&self, payload: T) -> Result<T, anyhow::Error> {
        let req_pkt = Packet::wrap_payload(payload, Default::default());
        let resp_pkt = self.port.b_req_resp(req_pkt).await?;
        Ok(resp_pkt.unwrap_payload())
    }
    pub async fn b_req_resp_flush(&self, payload: T) -> Result<T, anyhow::Error> {
        let req_pkt = Packet::wrap_payload(payload, Default::default());
        let resp_pkt = self.port.b_req_resp_flush(req_pkt).await?;
        Ok(resp_pkt.unwrap_payload())
    }
}
