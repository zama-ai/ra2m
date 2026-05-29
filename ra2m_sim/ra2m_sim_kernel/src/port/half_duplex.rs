//! Endpoint used for half-duplex communication -> {MasterPort<T>, SlavePort<T>}
//! Simple Half-Duplex port implementation for stream of data across modules
//! Support multiple outstanding request based on mpsc::channel capacity.
//!
use super::*;

use ra2m_sim_output::log::{Category, Verbosity};

use std::collections::LinkedList;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};

/// Master Endpoint definition that send payload over a communication channel
/// Support multiple outstanding packet
#[derive(Debug)]
pub struct MasterEndpoint<T> {
    name: String,
    props: Arc<module::Properties>,
    pub(super) pkt_inflight: Option<usize>,
    tx: Option<Box<mpsc::Sender<Packet<T>>>>,
}

impl<T: 'static + std::fmt::Debug> MasterEndpoint<T> {
    pub async fn send(&mut self, pkt: Packet<T>) {
        log!(|self| log::Category::Port, log::Verbosity::Trace => self.name, pkt);

        // TODO split the struct instead of cloning
        // => should satisfy the borrow checker
        let name = self.name.clone();
        let path = self.props.path();

        if let Some(tx) = &mut self.tx {
            match tx.try_send(pkt) {
                Ok(_) => {
                    // Packet sent directly.
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "Success, will yield");
                    tokio::task::yield_now().await;
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "back from yield");
                }
                Err(TrySendError::Full(pkt)) => {
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "Channel full, unregister from scheduler");
                    // Underlying channel is full, unregistered our process from the Hw Scheduler
                    // Then start a blocking send and rely on tokio runtime for woken-up
                    // After pkt correctly sent, register prc back in scheduler
                    scheduler::SchedulerAPI::lock_prc();
                    tx.send(pkt).await.expect("Port is closed");
                    scheduler::SchedulerAPI::unlock_evt();
                    scheduler::SchedulerAPI::unlock_prc();
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "Success, registered back in scheduler");
                }
                Err(TrySendError::Closed(_)) => {
                    panic!("{path}.{name}: Attempt to send packet across an closed channel")
                }
            }
        } else {
            panic!("{path}.{name}: Attempt to send packet across an unbinded channel")
        }
    }
}

impl<T: 'static> Endpoint for MasterEndpoint<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn properties(&self) -> &Arc<module::Properties> {
        &self.props
    }

    fn is_binded(&self) -> bool {
        self.tx.is_some()
    }

    fn pkt_inflight(&self) -> Option<usize> {
        self.pkt_inflight
    }

    fn connect_channels(
        &mut self,
        tx: Option<Box<dyn std::any::Any>>,
        rx: Option<Box<dyn std::any::Any>>,
    ) -> Result<(), anyhow::Error> {
        if self.is_binded() {
            return Err(EndpointError::AlrdyBind(
                format!("{}.{}", self.props.path(), self.name),
                self.is_binded(),
            )
            .into());
        }

        if !(tx.is_some() && rx.is_none()) {
            return Err(EndpointError::Handle(format!(
                "{}.{} expect a tx handle only [tx: {}, rx: {}].",
                self.props.path(),
                self.name,
                tx.is_some(),
                rx.is_some()
            ))
            .into());
        }

        // Downcast to generic interface type into concrete one
        match tx.unwrap().downcast::<mpsc::Sender<Packet<T>>>() {
            Ok(tx) => {
                self.tx = Some(tx);
                Ok(())
            }
            Err(err) => Err(EndpointError::Downcast(
                format!("{}.{}", self.props.path(), self.name),
                err.type_id(),
            )
            .into()),
        }
    }
}

/// Wrap a MasterEndpoint in a PortMutex for enable correct multiple concurrent access on it.
/// Useful since Port could be used concurrently by multiple task in a Module
/// Associated MasterEndpoint structure shouldn't be used without the locking around, port traits is
/// implemented for MasterPort not for MasterEndpoint.
#[derive(Debug)]
pub struct MasterPort<T>(pub PortMutex<MasterEndpoint<T>>);

impl<T: 'static> Port for MasterPort<T> {
    fn view_as_handle(&self) -> PortHandle<'_> {
        PortHandle::TxOnly(&self.0 as &PortMutex<dyn Endpoint>)
    }

    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error> {
        // Get tx endpoint
        let mut ep_tx = self
            .0
            .try_lock()
            .expect("Bind must be done before any Module init");

        // Get associated rx endpoint
        let ep_rx = match with_handle {
            PortHandle::RxOnly(ep_rx) => ep_rx,
            PortHandle::TxRx(_, ep_rx) => ep_rx,
            _ => {
                return Err(PortError::InvalEp(
                    format!("{}.{}", ep_tx.properties().path(), ep_tx.name()),
                    format!("{:?}", self.view_as_handle()),
                    format!("{with_handle:?}"),
                )
                .into());
            }
        };

        let mut ep_rx = ep_rx
            .try_lock()
            .expect("Bind must be done before any Module init");

        // Get inflight packet
        let pkt_inflight = match (ep_tx.pkt_inflight(), ep_rx.pkt_inflight()) {
            (Some(pkt_ift), None) | (None, Some(pkt_ift)) => pkt_ift,
            _ => {
                return Err(PortError::Inflight(
                    format!("{}.{}", ep_tx.properties().path(), ep_tx.name()),
                    ep_tx.pkt_inflight(),
                    format!("{}.{}", ep_rx.properties().path(), ep_rx.name()),
                    ep_rx.pkt_inflight(),
                )
                .into());
            }
        };

        // Allocate channel and connect ep to it
        let (tx, rx) = mpsc::channel::<Packet<T>>(pkt_inflight);
        let tx_boxed = Box::new(tx);
        let rx_boxed = Box::new(rx);
        ep_tx.connect_channels(Some(tx_boxed), None)?;
        ep_rx.connect_channels(None, Some(rx_boxed))?;
        Ok(())
    }
}

impl<T: 'static> PortNew for MasterPort<T> {
    fn new(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: Option<usize>,
        sid_width: Option<usize>,
    ) -> Self {
        assert!(
            sid_width.is_none(),
            "{}.{}: MasterPort don't support sid dispatch {:?}",
            props.path(),
            name,
            sid_width
        );
        Self(PortMutex::new(MasterEndpoint {
            name: name.to_string(),
            props,
            pkt_inflight,
            tx: None,
        }))
    }
}

/// Implement convenience functions to reduce the boilerplate code required for locking, casting
/// checking, logging, delay handling ...
impl<T> MasterPort<T>
where
    T: 'static + Send + TxStatus + std::fmt::Debug,
{
    /// Send packet
    /// Handle locking, check packet content and issue packet
    pub async fn send_pkt(&self, mut pkt: Packet<T>) -> Result<(), anyhow::Error> {
        // Lock underlying tx endpoint once
        let mut tx = self.0.lock().await;
        {
            // Enforce correct view of variable that cross await boundaries
            let req = pkt.payload();
            log!(|tx| log::Category::Protocol, log::Verbosity::Trace => tx.name, req);
            req.tx_check()?;
        }
        // Handle packet delay while underlying port is locked
        pkt.account_delay(None).await;
        // Send packet over tx endpoint
        tx.send(pkt).await;
        Ok(())
    }

    /// Send packet burst
    ///
    /// Lock tx endpoint, check and issue the specified list of packet.
    /// NB: This function block (return poll-pending) if the tx endpoint is already
    /// locked by another task or if the underlying channel is full.
    ///
    /// If the user required a non-blocking behavior (e.g. background task), function call must be
    /// wrapped in a dedicated hw process:
    ///  ``` no_build
    ///  spawn_prc!(self.send_pkt_burst( &self.port, pkt));
    ///  ```
    pub async fn send_pkt_burst(&self, mut pkt: Vec<Packet<T>>) -> Result<(), anyhow::Error> {
        // Lock underlying tx endpoint once
        let mut tx = self.0.lock().await;

        while let Some(mut p) = pkt.pop() {
            {
                // Enforce correct view of variable that cross await boundaries
                let req = p.payload();
                log!(|tx| log::Category::Protocol, log::Verbosity::Trace => req);
                req.tx_check()?;
            }
            // Handle packet delay while underlying port is locked
            p.account_delay(None).await;
            // Send packet over tx endpoint
            tx.send(p).await;
        }
        drop(tx);
        Ok(())
    }
}

impl<T> MasterPort<T>
where
    T: 'static + Send + std::fmt::Debug,
{
    /// Forward packet
    /// Handle locking and forwand packet (i.e. no checking)
    /// NB: This function block (return poll-pending) if the tx endpoint is already
    /// locked by another task or if the underlying channel is full.
    pub async fn fwd_pkt(&self, mut pkt: Packet<T>) {
        // Lock underlying tx endpoint once
        let mut tx = self.0.lock().await;
        {
            // Enforce correct view of variable that cross await boundaries
            let req = pkt.payload();
            log!(|tx| log::Category::Protocol, log::Verbosity::Trace => tx.name, req);
        }
        // Handle packet delay while underlying port is locked
        pkt.account_delay(None).await;
        // Send packet over tx endpoint
        tx.send(pkt).await;
    }

    /// Send packet burst
    ///
    /// Lock tx endpoint and issue the specified list of packet.
    /// NB: This function block (return poll-pending) if the tx endpoint is already
    /// locked by another task or if the underlying channel is full.
    ///
    /// If the user required a non-blocking behavior (e.g. background task), function call must be
    /// wrapped in a dedicated hw process:
    ///  ``` no_build
    ///  spawn_prc!(self.fwd_pkt_burst( &self.port, pkt_burst));
    ///  ```
    pub async fn fwd_pkt_burst(&self, mut pkt_burst: Vec<Packet<T>>) {
        // Lock underlying tx endpoint once
        let mut tx = self.0.lock().await;

        while let Some(mut pkt) = pkt_burst.pop() {
            {
                // Enforce correct view of variable that cross await boundaries
                let req = pkt.payload();
                log!(|tx| log::Category::Protocol, log::Verbosity::Trace => req);
            }
            // Handle packet delay while underlying port is locked
            pkt.account_delay(None).await;
            // Send packet over tx endpoint
            tx.send(pkt).await;
        }
        drop(tx);
    }
}

/// Slave Endpoint definition that receive payload over a communication channel
#[derive(Debug)]
pub struct SlaveEndpoint<T> {
    name: String,
    props: Arc<module::Properties>,
    pub(super) pkt_inflight: Option<usize>,
    rx: Option<Box<mpsc::Receiver<Packet<T>>>>,
}

impl<T: 'static + std::fmt::Debug> SlaveEndpoint<T> {
    pub async fn recv(&mut self) -> Packet<T> {
        let Self {
            name, props, rx, ..
        } = self;
        let path = props.path();

        // TODO rewrite with as_mut() and match ?
        if let Some(rx) = rx {
            let pkt = match rx.try_recv() {
                Ok(pkt) => {
                    // Packet received directly.
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "Success, will yield");
                    tokio::task::yield_now().await;
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "back from yield");
                    pkt
                }
                Err(TryRecvError::Empty) => {
                    log!(Category::Port, Verbosity::Debug
                        => path, name, scheduler::SchedulerAPI::global() => "Channel empty, unregister from scheduler");
                    // Underlying channel is empty, unregistered our process from the Hw Scheduler
                    // Then start a blocking recv and rely on tokio runtime for woken-up
                    // After pkt correctly received, register prc back in scheduler
                    scheduler::SchedulerAPI::lock_prc();
                    if let Some(pkt) = rx.recv().await {
                        scheduler::SchedulerAPI::unlock_evt();
                        scheduler::SchedulerAPI::unlock_prc();
                        log!(Category::Port, Verbosity::Debug
                            => path, name => "Success, registered back in scheduler");
                        pkt
                    } else {
                        panic!("{path}.{name}: Attempt to recv packet across an closed channel")
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    panic!("{path}.{name}: Attempt to recv packet across an closed channel")
                }
            };
            log!(props, log::Category::Port, log::Verbosity::Trace  => path, name, pkt);
            return pkt;
        }
        panic!("{path}.{name}: Attempt to received packet across an unbinded||closed channel")
    }

    pub async fn try_recv(&mut self) -> Option<Packet<T>> {
        let Self {
            name, props, rx, ..
        } = self;
        let path = props.path();

        // TODO rewrite with as_mut() and match ?
        if let Some(rx) = rx {
            match rx.try_recv() {
                Ok(pkt) => {
                    // Packet received directly.
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "Success, will yield");
                    tokio::task::yield_now().await;
                    log!(Category::Port, Verbosity::Debug
                        => path, name => "back from yield");
                    log!(props, log::Category::Port, log::Verbosity::Trace  => path, name, pkt);
                    Some(pkt)
                }
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    panic!("{path}.{name}: Attempt to recv packet across an closed channel",)
                }
            }
        } else {
            None
        }
    }
}

impl<T: 'static> Endpoint for SlaveEndpoint<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn properties(&self) -> &Arc<module::Properties> {
        &self.props
    }

    fn is_binded(&self) -> bool {
        self.rx.is_some()
    }

    fn pkt_inflight(&self) -> Option<usize> {
        self.pkt_inflight
    }

    fn connect_channels(
        &mut self,
        tx: Option<Box<dyn std::any::Any>>,
        rx: Option<Box<dyn std::any::Any>>,
    ) -> Result<(), anyhow::Error> {
        if self.is_binded() {
            return Err(EndpointError::AlrdyBind(
                format!("{}.{}", self.props.path(), self.name),
                self.is_binded(),
            )
            .into());
        }

        if !(tx.is_none() && rx.is_some()) {
            return Err(EndpointError::Handle(format!(
                "{}.{} expect a rx handle only [tx: {}, rx: {}].",
                self.props.path(),
                self.name,
                tx.is_some(),
                rx.is_some()
            ))
            .into());
        }

        // Downcast to generic interface type into concrete one
        match rx.unwrap().downcast::<mpsc::Receiver<Packet<T>>>() {
            Ok(rx) => {
                self.rx = Some(rx);
                Ok(())
            }
            Err(err) => Err(EndpointError::Downcast(
                format!("{}.{}", self.props.path(), self.name),
                err.type_id(),
            )
            .into()),
        }
    }
}

/// Wrap a SlaveEndpoint in a PortMutex for enable correct multiple concurrent access on it.
/// Useful since Port could be used concurrently by multiple task in a Module
/// Associated SlaveEndpoint structure shouldn't be used without the locking around, port traits is
/// implemented for SlavePort not for SlaveEndpoint.
#[derive(Debug)]
pub struct SlavePort<T>(pub PortMutex<SlaveEndpoint<T>>);

impl<T> Deref for SlavePort<T> {
    type Target = PortMutex<SlaveEndpoint<T>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: 'static> Port for SlavePort<T> {
    fn view_as_handle(&self) -> PortHandle<'_> {
        PortHandle::RxOnly(&self.0 as &PortMutex<dyn Endpoint>)
    }

    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error> {
        // Get rx endpoint
        let mut ep_rx = self
            .0
            .try_lock()
            .expect("Bind must be done before any Module init");

        // get associated tx endpoint
        let ep_tx = match with_handle {
            PortHandle::TxOnly(ep_tx) => ep_tx,
            PortHandle::TxRx(ep_tx, _) => ep_tx,
            _ => {
                return Err(PortError::InvalEp(
                    format!("{}.{}", ep_rx.properties().path(), ep_rx.name()),
                    format!("{:?}", self.view_as_handle()),
                    format!("{with_handle:?}"),
                )
                .into());
            }
        };
        let mut ep_tx = ep_tx
            .try_lock()
            .expect("Bind must be done before any Module init");

        // Get inflight packets
        let pkt_inflight = match (ep_tx.pkt_inflight(), ep_rx.pkt_inflight()) {
            (Some(pkt_ift), None) | (None, Some(pkt_ift)) => pkt_ift,
            _ => {
                return Err(PortError::Inflight(
                    format!("{}.{}", ep_tx.properties().path(), ep_tx.name()),
                    ep_tx.pkt_inflight(),
                    format!("{}.{}", ep_rx.properties().path(), ep_rx.name()),
                    ep_rx.pkt_inflight(),
                )
                .into());
            }
        };

        // Allocate channel and connect ep to it
        let (tx, rx) = mpsc::channel::<Packet<T>>(pkt_inflight);
        let tx_boxed = Box::new(tx);
        let rx_boxed = Box::new(rx);
        ep_tx.connect_channels(Some(tx_boxed), None)?;
        ep_rx.connect_channels(None, Some(rx_boxed))?;
        Ok(())
    }
}

impl<T: 'static> PortNew for SlavePort<T> {
    fn new(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: Option<usize>,
        sid_width: Option<usize>,
    ) -> Self {
        assert!(
            sid_width.is_none(),
            "{}.{}: SlavePort don't support sid dispatch {:?}",
            props.path(),
            name,
            sid_width
        );
        Self(PortMutex::new(SlaveEndpoint {
            name: name.to_string(),
            props,
            pkt_inflight,
            rx: None,
        }))
    }
}

/// Implement convenience functions to reduce the boilerplate code required for locking
/// and handling packet delay
/// `_ep` suffix means that the receiving port is considered as the communication endpoint (end of
/// life of the packet). Thus, forcing the packet delay consumption whatever timing mode is used.
impl<T> SlavePort<T>
where
    T: 'static + Send + RxStatus + std::fmt::Debug,
{
    /// Wait a packet with given uid
    /// Use in endpoint (i.e. final stages, packet will be drop after use)
    /// Thus force delay handling and generate trace entry
    /// Also do sanity check on packet and payload
    ///
    /// NB: This function block (return poll-pending) if the rx endpoint is already
    /// locked by another task or if the underlying channel is empty.
    pub async fn wait_pkt_ep(&self, uid: Option<usize>) -> Result<Packet<T>, anyhow::Error>
    where
        T: Trace,
    {
        // Received associated response over rx endpoint
        let mut rx = self.0.lock().await;
        let mut pkt = {
            let pkt = rx.recv().await;
            match uid {
                None => pkt,
                Some(u) => {
                    if pkt.uid() == u {
                        pkt
                    } else {
                        log!(|rx| log::Category::Protocol, log::Verbosity::Warning
                            => rx.name, pkt.uid(), uid
                            => "Packet UID mismatch, reordering occur due to multiple outstanding or reset. In case of multiple outstanding, you should use a more evolved req/resp handling (Cf. req_burst/wait_pkt)");
                        return Err(PacketError::UidMismatch(pkt.uid(), u).into());
                    }
                }
            }
        };

        pkt.handle_delay().await;
        let payload = pkt.payload_mut();
        log!(|rx| log::Category::Protocol, log::Verbosity::Trace => rx.name, payload);
        trace!(|rx| trace::Kind::Payload => [rx.name] payload);
        // Check payload state
        payload.rx_check()?;
        drop(rx);

        // Check packet properties
        // -> untagged port mustn't handle pkt with Some(sid) as endpoint
        if let Some(sid) = pkt.sid() {
            Err(PacketError::UntaggedSid(*sid).into())
        } else {
            Ok(pkt)
        }
    }

    /// Wait a burst of packet
    /// Use in endpoint (i.e. final stages, packet will be drop after use)
    /// Thus force delay handling and generate trace entry
    ///
    /// Response are reorder based on packet uid. Packet uid are generated almost in order
    /// (except at the wrapping point). Thus, ordering the recv response by uid must enforce
    /// the same ordering as the issued request.
    /// FIXME Handle uid wrapping case
    ///
    /// Sanity check are also done on packet and payload
    ///
    /// NB: This function block (return poll-pending) if the rx endpoint is already
    /// locked by another task or if the underlying channel is empty.
    pub async fn wait_pkt_burst_ep(
        &self,
        resp_cnt: usize,
    ) -> LinkedList<(Packet<T>, Result<(), anyhow::Error>)>
    where
        T: Trace,
    {
        let check_status = |pkt: &Packet<T>| {
            // Check packet properties
            // -> untagged port mustn't handle pkt with Some(sid) as endpoint
            if let Some(sid) = pkt.sid() {
                return Err(PacketError::UntaggedSid(*sid).into());
            }

            // Check Payload properties
            match pkt.payload().rx_check() {
                Ok(_) => Ok(()),
                Err(err) => Err(err),
            }
        };

        // Create linked lists
        let mut resp = LinkedList::new();

        // Lock underlying rx endpoint once
        let mut rx = self.0.lock().await;
        for _ in 0..resp_cnt {
            // Recv next response and handle associated delay
            let mut pkt = rx.recv().await;
            pkt.handle_delay().await;
            let payload = pkt.payload_mut();
            log!(|rx| log::Category::Protocol, log::Verbosity::Trace => rx.name, payload);
            trace!(|rx| trace::Kind::Payload => [rx.name] payload);
            let pkt_status = check_status(&pkt);

            // Push in linked list in packet order
            let mut llc = resp.cursor_front_mut();
            loop {
                match llc.current() {
                    None => {
                        // reach end simply push back
                        resp.push_back((pkt, pkt_status));
                        break;
                    }
                    Some((llc_pkt, _)) => {
                        if pkt < *llc_pkt {
                            llc.insert_before((pkt, pkt_status));
                            break;
                        } else {
                            // Point to the next element
                            llc.move_next();
                        }
                    }
                }
            }
        }
        resp
    }
}

/// Implement convenience functions to reduce the boilerplate code required for locking
/// and handling packet delay
/// Communication middlepoint (i.e. packet will be update and forward):
/// -> Account for delay
/// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
///
impl<T> SlavePort<T>
where
    T: 'static + Send + std::fmt::Debug,
{
    /// Wait a packet
    /// Use in middlepoint (i.e. packet will be update and forward)
    /// -> Account for delay
    /// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
    pub async fn wait_pkt(&self) -> Packet<T> {
        // Received associated response over rx endpoint
        let mut rx = self.0.lock().await;
        let mut pkt = rx.recv().await;

        pkt.account_delay(None).await;
        let payload = pkt.payload();
        log!(|rx| log::Category::Protocol, log::Verbosity::Trace => (&rx.name, payload));
        drop(rx);
        pkt
    }

    /// Wait a burst of packet
    /// Use in middlepoint (i.e. packet will be update and forward)
    /// -> Account for delay
    /// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
    pub async fn wait_pkt_burst(&self, resp_cnt: usize) -> LinkedList<Packet<T>> {
        // Create linked lists
        let mut resp = LinkedList::new();

        // Lock underlying rx endpoint once
        let mut rx = self.0.lock().await;
        for _ in 0..resp_cnt {
            // Recv next response and handle associated delay
            let mut pkt = rx.recv().await;
            pkt.account_delay(None).await;
            let payload = pkt.payload();
            log!(|rx| log::Category::Protocol, log::Verbosity::Trace => rx.name, payload);

            // Push in linked list in packet order
            let mut llc = resp.cursor_front_mut();
            loop {
                match llc.current() {
                    None => {
                        // reach end simply push back
                        resp.push_back(pkt);
                        break;
                    }
                    Some(llc_pkt) => {
                        if pkt < *llc_pkt {
                            llc.insert_before(pkt);
                            break;
                        } else {
                            // Point to the next element
                            llc.move_next();
                        }
                    }
                }
            }
        }
        resp
    }
}
