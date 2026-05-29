//! Port used for full-duplex communication:
//!  * ReqRespPort -> Simple RequestResponse port
//!  * DispatchPort -> Tagged RequestResponse port. i.e packet are tagged with session id,
//!                  communication flux are split on the response side in multiple session
//!                  queues.
//!
//! These ports are build on top of two half_duplex ports and dispatcher for session handling.

use super::*;

/// Full-Duplex port definition that send request and receive response over communication channel
/// support multiple outstanding payload.
/// Requester/Receiver share the same behavior, except for the binding phase. Indeed to enforce
/// that the max_outstanding is equal on both ends, The requester is the only one in charge of
/// setting this channel property (eg. it lead the binding).
#[derive(Debug, Getters)]
pub struct ReqRespPort<T> {
    #[getset(get = "pub")]
    tx: MasterPort<T>,
    #[getset(get = "pub")]
    rx: SlavePort<T>,
}

impl<T: 'static> PortNew for ReqRespPort<T> {
    fn new(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: Option<usize>,
        sid_width: Option<usize>,
    ) -> Self {
        assert!(
            sid_width.is_none(),
            "{}.{}: ReqRespPort don't support sid dispatch [get {:?}]",
            props.path(),
            name,
            sid_width
        );

        let name_tx = format!("{name}.tx");
        let name_rx = format!("{name}.rx");
        ReqRespPort {
            tx: PortNew::new(&name_tx, props.clone(), pkt_inflight, None),
            rx: PortNew::new(&name_rx, props, pkt_inflight, None),
        }
    }
}

/// Implement utility function for Requester endpoint
/// i.e. Initiator -> ... -> Last endpoint
impl<T> ReqRespPort<T>
where
    T: 'static + TxStatus + RxStatus + Send + std::fmt::Debug + Trace,
{
    /// Simple request response without multiple outstanding support
    ///
    /// Forge a well-formed request and issue it.
    /// Wait for matching response
    /// Return error in case of unmatched uid or invalid response status
    pub async fn b_req_resp(&self, pkt: Packet<T>) -> Result<Packet<T>, anyhow::Error> {
        // Send request
        let uid = pkt.uid();
        self.tx.send_pkt(pkt).await?;

        // Wait matching response
        self.rx.wait_pkt_ep(Some(uid)).await
    }

    /// Simple request response without multiple outstanding support
    ///
    /// Forge a well-formed request and issue it.
    /// Wait for matching response, unmatched packet are dropped (issue warning)
    /// Check response Status
    ///
    /// This variant could be useful to flush old packet in port (e.g. after a simulated reset)
    pub async fn b_req_resp_flush(&self, pkt: Packet<T>) -> Result<Packet<T>, anyhow::Error> {
        // Send request
        let uid = pkt.uid();
        self.tx.send_pkt(pkt).await?;

        // Wait matching response
        loop {
            match self.rx.wait_pkt_ep(Some(uid)).await {
                Ok(pkt) => break Ok(pkt),
                Err(err) => match err.downcast_ref::<PacketError>() {
                    Some(PacketError::UidMismatch(_, _)) => continue,
                    _ => break Err(err),
                },
            }
        }
    }

    /// Issue burst request in background and wait for associated response
    /// Request issued in background to enable burst size bigger that port outstanding
    /// Response awaited in foreground
    pub async fn b_req_resp_burst(
        &self,
        pkt_burst: Vec<Packet<T>>,
    ) -> LinkedList<(Packet<T>, Result<(), anyhow::Error>)> {
        let burst_len = pkt_burst.len();

        // Spawn background process for issuing request
        // crate::spawn_prc!(self.pkt_burst(pkt_burst));
        // FIXME slice the burst instead, this spawn crash the compiler oO
        match self.tx.send_pkt_burst(pkt_burst).await {
            Ok(_) => {}
            Err(err) => {
                panic!("Error encounter while issuing a b_req_resp_burst {err}");
            }
        }

        // Wait associated responses
        self.rx.wait_pkt_burst_ep(burst_len).await
    }
}

/// Implement utility function for Responder endpoint
/// i.e. ... -> Handle & Forward -> ...
impl<T> ReqRespPort<T>
where
    T: 'static + Send + TxStatus + RxStatus + std::fmt::Debug + Trace,
{
    /// Wait for request, forge response and send it back
    ///
    /// NB: This function block (return poll-pending) if the rx/tx endpoint is already
    /// locked by another task or if the underlying channel is empty/full.
    ///
    /// Error management: In case of error, error code should be encoded in the response.
    /// This enable the error handling to occur in the master side.
    /// (ex. for MemBus use the mode field to encode error status
    pub async fn wait_req_forge_resp<F: FnOnce(&mut T) -> Option<time::Tick>>(
        &self,
        forge_resp: F,
    ) -> Result<(), anyhow::Error> {
        // Wait for request
        let mut pkt = self.rx.wait_pkt().await;
        let req = pkt.payload_mut();

        // Check initial status
        req.tx_check()?;

        // Forge response
        let delay = forge_resp(req);
        req.rx_check()?;
        if let Some(dly) = delay {
            pkt.append_delay(dly);
        }

        // Send response packet over tx endpoint
        self.tx.fwd_pkt(pkt).await;
        Ok(())
    }
}

impl<T: 'static> Port for ReqRespPort<T> {
    fn view_as_handle(&self) -> PortHandle<'_> {
        let handle_tx = self.tx.view_as_handle();
        let handle_rx = self.rx.view_as_handle();
        match (handle_tx, handle_rx) {
            (PortHandle::TxOnly(ep_tx), PortHandle::RxOnly(ep_rx)) => {
                PortHandle::TxRx(ep_tx, ep_rx)
            }
            _ => {
                panic!("couldn't be reached")
            }
        }
    }

    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error> {
        self.tx.bind(with_handle)?;
        self.rx.bind(with_handle)?;
        Ok(())
    }
}

/// Tagged Full-Duplex Port definition that send request and receive response over communication channel
/// support multiple outstanding payload.
/// On the receiver side, a dipatcher switch the communication flux in multiple session queues.
/// This enable multiple tasks to merge their packet traffic in the same port without mixing the
/// associated session (i.e. all task only received the response associated with the issued
/// request).
///
/// Requester/Receiver share the same behavior, except for the binding phase. Indeed to enforce
/// that the max_outstanding is equal on both ends, The requester is the only one in charge of
/// setting these channels property (eg. it lead the binding).
#[derive(Debug, Getters)]
pub struct DispatchPort<T> {
    #[getset(get = "pub")]
    tx: MasterPort<T>,
    #[getset(get = "pub")]
    drx: Dispatch<T>,
}

impl<T: 'static> PortNew for DispatchPort<T> {
    fn new(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: Option<usize>,
        sid_width: Option<usize>,
    ) -> Self {
        let name_tx = format!("{name}.tx");
        let inflight = *pkt_inflight.as_ref().expect("Dispatch inflight packet must be set at creation. Couldn't be deferred to elaboration phase");
        let sid_width = *sid_width.as_ref().unwrap_or_else(|| {
            panic!(
                "{}.{}: DispatchPort required sid dispatch [get {:?}]",
                props.path(),
                name,
                sid_width
            )
        });
        DispatchPort {
            tx: PortNew::new(&name_tx, props.clone(), pkt_inflight, None),
            drx: Dispatch::new(name, props, inflight, sid_width),
        }
    }
}

/// Implement utility function for Requester endpoint
/// i.e. Initiator -> ... -> Last endpoint
impl<T> DispatchPort<T>
where
    T: 'static + TxStatus + RxStatus + Send + std::fmt::Debug + Trace,
{
    /// Simple request response without multiple outstanding support
    ///
    /// Forge a well-formed request and issue it.
    /// Wait for matching response, unmatched packet are dropped (issue warning)
    /// Check response Status
    pub async fn b_req_resp(&self, pkt: Packet<T>) -> Result<Packet<T>, anyhow::Error> {
        let sid = match *pkt.sid() {
            Some(sid) => sid,
            None => return Err(PacketError::TaggedNoSid.into()),
        };
        // Send request
        self.tx.send_pkt(pkt).await?;

        // Wait matching response
        self.drx.wait_pkt_ep(sid).await
    }

    /// Issue burst request in background and wait for associated response
    /// Request issued in background to enable burst size bigger that port outstanding
    /// Response awaited in foreground
    /// NB: sid is given as extra args to enforce that all packet have the same sid
    pub async fn b_req_resp_burst(
        &self,
        sid: usize,
        mut pkt_burst: Vec<Packet<T>>,
    ) -> std::collections::VecDeque<(Packet<T>, Result<(), anyhow::Error>)> {
        // Enforce that all pkt in burst use the same sid
        pkt_burst.iter_mut().for_each(|pkt| {
            pkt.set_sid(Some(sid));
        });

        let burst_len = pkt_burst.len();
        // Spawn background process for issuing request
        // crate::spawn_prc!(self.pkt_burst(pkt_burst));
        // FIXME slice the burst instead, this spawn crash the compiler oO
        match self.tx.send_pkt_burst(pkt_burst).await {
            Ok(_) => {}
            Err(err) => {
                panic!("Error encounter while issuing a b_req_resp_burst {err}");
            }
        }

        // Wait associated responses
        // TODO how to handle delay properly
        self.drx.wait_pkt_burst_ep(sid, burst_len).await
    }
}

/// Implement utility function for Responder endpoint
/// i.e. ... -> Handle & Forward -> ...
impl<T> DispatchPort<T>
where
    T: 'static + Send + TxStatus + RxStatus + std::fmt::Debug + Trace,
{
    /// Wait for request, forge response and send it back
    ///
    /// NB: This function block (return poll-pending) if the rx/tx endpoint is already
    /// locked by another task or if the underlying channel is empty/full.
    ///
    /// Error management: In case of error, error code should be encoded in the response.
    /// This enable the error handling to occur in the master side.
    /// (ex. for MemBus use the mode field to encode error status
    pub async fn wait_req_forge_resp<F: FnOnce(&mut T) -> Option<time::Tick>>(
        &self,
        sid: usize,
        forge_resp: F,
    ) -> Result<(), anyhow::Error> {
        // Wait for request
        let mut pkt = self.drx.wait_pkt(sid).await;
        let req = pkt.payload_mut();

        // Check initial status
        req.tx_check()?;

        // Forge response
        let delay = forge_resp(req);
        req.rx_check()?;
        if let Some(dly) = delay {
            pkt.append_delay(dly);
        }
        // Send response packet over tx endpoint
        self.tx.fwd_pkt(pkt).await;
        Ok(())
    }
}

impl<T: 'static> Port for DispatchPort<T> {
    fn view_as_handle(&self) -> PortHandle<'_> {
        let handle_tx = self.tx.view_as_handle();
        let handle_rx = self.drx.port().view_as_handle();
        match (handle_tx, handle_rx) {
            (PortHandle::TxOnly(ep_tx), PortHandle::RxOnly(ep_rx)) => {
                PortHandle::TxRx(ep_tx, ep_rx)
            }
            _ => {
                panic!("couldn't be reached")
            }
        }
    }

    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error> {
        self.tx.bind(with_handle)?;
        self.drx.port().bind(with_handle)?;
        Ok(())
    }
}
