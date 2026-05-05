//! Implement Session handling for Packet
//!
//! Enable to mixed multiple stream of packet across the same port with ease.
//! Upon creation packet are attached to a given session (eg. sid field).
//! A dedicated hw prc is attach to the rx endpoint and split the receive stream of packet across
//! the number of supported sessions.
//!

use super::*;

pub use std::collections::LinkedList;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Queue
/// Provide async enqueue/dequeue operation to store the received Packet.
/// Internally rely on linked list to reorder the Packet stream.
/// Hw process notification is build upon scheduler events.
#[derive(Debug)]
struct Queue<T> {
    depth: usize,
    enq_evt_name: String,
    deq_evt_name: String,
    ll: Mutex<LinkedList<Packet<T>>>,
}

impl<T> Queue<T> {
    fn new(depth: usize, qid: usize, evt_bname: &str) -> Self {
        let enq_evt_name = format!("{evt_bname}.enqueue_{qid}");
        let deq_evt_name = format!("{evt_bname}.dequeue_{qid}");
        Self {
            depth,
            enq_evt_name,
            deq_evt_name,
            ll: Mutex::new(LinkedList::new()),
        }
    }

    /// Enqueue Packet
    /// Packet are ordered in the underlying linked list by uid
    /// Return poll::Pending if maximum queue depth is reached
    async fn enqueue(&self, pkt: Packet<T>) {
        let mut ll = loop {
            // Wait that enough space is available
            {
                // Used extra scope to circumvent Send-ness analysis limitation of future
                let ll = self.ll.lock().unwrap();
                if ll.len() < self.depth {
                    break ll;
                }
            }

            // Queue is full, wait on the associated event
            event::Event::wait(&self.deq_evt_name).await;
        };

        // Push in linked list in packet order
        let mut llc = ll.cursor_front_mut();
        loop {
            match llc.current() {
                None => {
                    // reach end simply push back
                    ll.push_back(pkt);
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

        // Notify pending dequeue if any
        event::Event::triggered(&self.enq_evt_name, None);
    }

    /// Dequeue burst of packet
    /// Return poll::Pending if queue hasn't enough elements
    async fn dequeue(&self, len: usize) -> LinkedList<Packet<T>> {
        let mut ll = loop {
            // Wait that enough element are present in the queue
            {
                // Used extra scope to circumvent Send-ness analysis limitation of future
                let ll = self.ll.lock().unwrap();
                if ll.len() >= len {
                    break ll;
                }
            }
            // Not enough element in queue, wait on the associated event
            event::Event::wait(&self.enq_evt_name).await;
        };

        // Sliced the requested number of element from the list
        // NB: Extract the list head and keep the remaining tail (inverse of the split_off function)
        let tail = ll.split_off(len);
        let head = ll.split_off(0);
        *ll = tail;

        // Notify pending enqueue if any
        event::Event::triggered(&self.deq_evt_name, None);
        head
    }

    /// Flush queue content
    fn flush(&self) {
        self.ll.lock().unwrap().clear();
    }
}

#[derive(Debug)]
struct DispatchInner<T> {
    name: String,
    props: Arc<module::Properties>,
    port: SlavePort<T>,
    queue: Vec<Queue<T>>,
}

/// Enable proper logging/tracing without having to lock underlying PortEndpoint
/// Useful since dispatch logging is delay into dequeue stage
impl<T> DispatchInner<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn properties(&self) -> &Arc<module::Properties> {
        &self.props
    }
}

impl<T> DispatchInner<T>
where
    T: 'static + Send + std::fmt::Debug,
{
    /// Dedicated function that must be spawned as HW process (Cf. Dispatch::init).
    /// Poll the associated endpoint and dispatch received packet to the
    /// corresponding session queue.
    async fn poll_port(self: Arc<Self>) {
        loop {
            // Wait packet
            let pkt = self.port.wait_pkt().await;

            // Enqueue in matching sid queue
            // Default to queue 0 in case sid isn't set
            let sid = pkt.sid().unwrap_or(0);
            self.queue[sid].enqueue(pkt).await;
        }
    }
}

#[derive(Debug)]
pub struct Dispatch<T> {
    width: usize,
    is_spawned: AtomicBool,
    inner: Arc<DispatchInner<T>>,
}

impl<T: 'static> Dispatch<T> {
    pub fn new(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: usize,
        sid: usize,
    ) -> Self {
        let queue_depth = vec![pkt_inflight; sid];
        Self::from_depth(name, props, pkt_inflight, queue_depth)
    }

    pub fn from_depth(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: usize,
        depth: Vec<usize>,
    ) -> Self {
        // Allocate dispatch queues
        let evt_bname = format!("{}.{}", props.path(), name);
        let mut queue = Vec::new();
        for (i, d) in depth.iter().enumerate() {
            queue.push(Queue::new(*d, i, &evt_bname));
        }

        let name_rx = format!("{name}.drx");
        Self {
            width: depth.len(),
            is_spawned: AtomicBool::new(false),
            inner: Arc::new(DispatchInner {
                name: name.to_string(),
                port: port::PortNew::new(&name_rx, props.clone(), Some(pkt_inflight), None),
                queue,
                props,
            }),
        }
    }

    pub fn port(&self) -> &SlavePort<T> {
        &self.inner.port
    }

    pub fn flush_all(&mut self) {
        for queue in self.inner.queue.iter() {
            queue.flush();
        }
    }
}

impl<T> Dispatch<T>
where
    T: 'static + Send + std::fmt::Debug,
{
    /// Spawned Hw process to poll the associated port and dispatch packet over queues
    pub fn init(&self) {
        // Prevent multiple spawn
        if !self.is_spawned.load(Ordering::SeqCst) {
            let asc = self.inner.clone();
            crate::spawn_prc!(asc.poll_port());
            self.is_spawned.store(true, Ordering::SeqCst);
        }
    }
}

/// Implement convenience functions to reduce the boilerplate code required for locking
/// and handling packet delay
/// `_ep` suffix means that the receiving port is considered as the communication endpoint (end of
/// life of the packet). Thus, forcing the packet delay consumption whatever timing mode is used.
impl<T> Dispatch<T>
where
    T: 'static + Send + RxStatus + std::fmt::Debug + Trace,
{
    /// Wait a packet belonging to session sid
    /// Use in endpoint (i.e. final stages, packet will be drop after use)
    /// Thus force delay handling and generate trace entry
    /// Also do sanity check on packet and payload
    pub async fn wait_pkt_ep(&self, sid: usize) -> Result<Packet<T>, anyhow::Error> {
        // Retrieved packet and handle delay
        let mut pkt = self.wait_pkt(sid).await;
        pkt.handle_delay().await;
        let payload = pkt.payload_mut();

        let self_inner = &self.inner;
        log!(|self_inner| log::Category::Protocol, log::Verbosity::Trace => self_inner.name(), payload);
        trace!(|self_inner| trace::Kind::Payload => [self_inner.name()] payload);

        // Check payload state
        payload.rx_check()?;

        // Check packet properties
        // -> Tagged port mustn't handle pkt without sid
        // TODO fine a better way to handle this error. Currently packet are redirected in queue
        // 0 and error is only raised on dequeue with flavor _ep
        if pkt.sid().is_none() {
            Err(PacketError::TaggedNoSid.into())
        } else {
            Ok(pkt)
        }
    }

    /// Wait a burst of packet belonging to session sid
    /// Use in middlepoint (i.e. packet will be update and forward)
    /// -> Account for delay
    /// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
    pub async fn wait_pkt_burst_ep(
        &self,
        sid: usize,
        nb_pkt: usize,
    ) -> std::collections::VecDeque<(Packet<T>, Result<(), anyhow::Error>)> {
        let check_status = |pkt: &Packet<T>| {
            // Check packet properties
            if pkt.sid().is_none() {
                return Err(PacketError::TaggedNoSid.into());
            }

            // Check Payload properties
            match pkt.payload().rx_check() {
                Ok(_) => Ok(()),
                Err(err) => Err(err),
            }
        };

        let mut wrap_pkt = std::collections::VecDeque::with_capacity(nb_pkt);
        let pkt_ll = self.wait_pkt_burst(sid, nb_pkt).await;
        for pkt in pkt_ll.into_iter() {
            let status = check_status(&pkt);
            wrap_pkt.push_back((pkt, status));
        }
        wrap_pkt
    }
}

/// Implement convenience functions to reduce the boilerplate code required for locking
/// and handling packet delay
/// Communication middlepoint (i.e. packet will be update and forward):
/// -> Account for delay
/// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
impl<T> Dispatch<T>
where
    T: 'static + Send + std::fmt::Debug + Trace,
{
    /// Wait a packet belonging to session sid
    /// Use in middlepoint (i.e. packet will be update and forward)
    /// -> Account for delay
    /// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
    pub async fn wait_pkt(&self, sid: usize) -> Packet<T> {
        let mut ll = self.wait_pkt_burst(sid, 1).await;
        ll.pop_front().unwrap()
    }

    /// Wait a burst of packet belonging to session sid
    /// Use in middlepoint (i.e. packet will be update and forward)
    /// -> Account for delay
    /// -> No trace entries were generated, no checks were done in middlepoint pkt/payload
    pub async fn wait_pkt_burst(&self, sid: usize, nb_pkt: usize) -> LinkedList<Packet<T>> {
        assert!(
            self.is_spawned.load(Ordering::SeqCst),
            "Attempt to access uninit Dispatch. Dispatch must be init to poll associated ep {}",
            self.inner.port.0.lock().await.name()
        );

        assert!(
            self.width > sid,
            "Attempt to access session {} from Dispatch[{}]. Invalid sid requested",
            sid,
            self.width
        );
        self.inner.queue[sid].dequeue(nb_pkt).await
    }
}
