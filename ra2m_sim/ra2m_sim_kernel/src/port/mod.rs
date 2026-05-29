//! Implement Point to Point communication protocol between Hw Modules
//! This module provide communication primitives that must be used by library module to implement
//! complex communication patterns.
//!
//! # trait Endpoint
//!   Trait that enable generic connection between port implementations.
//!   This trait rely on the generic packet format to connect two port together.
//!   A check is done at elaboration time (based on Any trait)to validate the port compatibility
//!
//! # trait Port
//!   Enable port binding from the wrapped view (eg. `tokio::sync::Mutex<dyn Port>`)
//!   Indeed port are globally shared and must be wrapped in a lock enable correct
//!   multithreading
//!   Cf. `half_duplex::MasterEndpoint<T> vs half_duplex::MasterPort<T>`
//!
//! # Data:
//!  Endpoint exchange Packet. Packet is a generic state around a Payload.
//!  Data communication rely on async channel to be polled efficiently.
//!
//!  _NB_: async channel use std::future as opposed custom kernel::{delay,event} futures.
//!  roughly explained:
//!    - std::future handle events in the host timespace
//!    - kernel::{delay, event} futures handle events in the simulated timespace
//!
//! # Timing Mode
//!  Two timing flavors are supported:
//!   - Loosely timed {LT}: Delay are accumulated in Packet and awaited only once
//!   - Approximately timed {AT}: Delay are awaited at every stages
//!
//! __NB'__: The same behavior for packet handling must be shared by AT/LT.
//!  By this way, the user write behavior only once to obtain both modes.
//!  (Cf. handle_delay/ account_delay/ append_delay methods on Packet ).
//!
//! __NB'__: The timing flavor could be toggled at runtime (eg. LT for bootup then AT for Roi)
//!
//! # Wrapping in tokio::sync::Mutex
//! Ports represent the boundary of modules and thus will be shared by all Hw process inside
//! a module. A common pattern is to wrapped them in Async Mutex.
//! Async Mutex are required here since the lock should be kept across await point depending on the
//! underlying channel status.
//! Full duplex ports already rely on wrapped half duplex ports (enable to lock master/ slave layer
//! independently)
//!

use super::*;
use log::{Category, Verbosity};
use protocol::Trace;
use ra2m_sim_containers::prelude::*;
use ra2m_sim_output::prelude::*;

/// Packet and associated types
pub mod packet;
pub use self::packet::{Packet, PacketError, PacketOptions};

/// Packet dispatcher for tagged communication
pub mod dispatch;
use dispatch::{Dispatch, LinkedList};

/// Half-Duplex port implementation
pub mod half_duplex;
pub use self::half_duplex::{MasterPort, SlavePort};

/// Full-Duplex port implementation
pub mod full_duplex;
pub use self::full_duplex::{DispatchPort, ReqRespPort};

#[cfg(test)]
mod units_tests;

use getset::Getters;
use std::any::TypeId;
use std::ops::Index;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, MutexGuard, TryLockError};

/// Trait that enable generic connection between port implementations.
/// This trait rely on dynamic typing and type reflection to check the port compatibility at
/// runtime.
pub trait Endpoint {
    fn name(&self) -> &str;
    fn properties(&self) -> &Arc<module::Properties>;
    fn is_binded(&self) -> bool;
    fn pkt_inflight(&self) -> Option<usize>;
    fn connect_channels(
        &mut self,
        tx: Option<Box<dyn std::any::Any>>,
        rx: Option<Box<dyn std::any::Any>>,
    ) -> Result<(), anyhow::Error>;
}

/// EndpointError type
/// Describe common error that could occurred with Port /Endpoint management
#[derive(Error, Debug)]
pub enum EndpointError {
    #[error("{0} -> Unsupported handles configuration")]
    Handle(String),
    #[error("{0}::{1} -> Endpoint is already bound")]
    AlrdyBind(String, bool),
    #[error("{0} -> Channel downcast from invalid  type {1:?}")]
    Downcast(String, TypeId),
}

/// Generic Handle for Endpoints
/// Enum that could contain various Endpoint handles configuration.
/// Each handle are guarded by a async mutex. Enum entry match with port kind:
///  * TxOnly   => MasterPort endpoint
///  * RxOnly   => SlavePort endpoint
///  * TxRxReq  => ReqRespPort (or DispatchPort) endpoint with Request flavor
///  * TxRxResp => ReqRespPort (or DispatchPort) endpoint with Response flavor
///
///  Gathering port handle in a enum enable custom splitting and gathering of port handle.
///  Like binding an ReqRespPort on two distinct port instances of port (eg. Master, Slave) or the
///  opposite
#[derive(Clone, Copy)]
pub enum PortHandle<'a> {
    TxOnly(&'a PortMutex<dyn Endpoint>),
    RxOnly(&'a PortMutex<dyn Endpoint>),
    TxRx(&'a PortMutex<dyn Endpoint>, &'a PortMutex<dyn Endpoint>),
}

impl<'a> std::fmt::Debug for PortHandle<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TxOnly(_) => write!(f, "TxOnly")?,
            Self::RxOnly(_) => write!(f, "RxOnly")?,
            Self::TxRx(_, _) => write!(f, "TxRx")?,
        }
        Ok(())
    }
}

/// Blanket implementation for any type that is Port
impl<'a, T: Port> From<&'a T> for PortHandle<'a> {
    fn from(port: &'a T) -> Self {
        port.view_as_handle()
    }
}

/// Blanket implementation for any reference of dyn PortLock
impl<'a> From<&'a dyn Port> for PortHandle<'a> {
    fn from(port: &'a dyn Port) -> Self {
        port.view_as_handle()
    }
}

/// PortError type
/// Describe common error that could occurred with Port /Endpoint management
#[derive(Error, Debug)]
pub enum PortError {
    #[error("{0}::{1:?} <=> {2}::{3:?} -> [Only/ AtLeast] one endpoint must setup the pkt_inflight field")]
    Inflight(String, Option<usize>, String, Option<usize>),
    #[error("{0}::{1} <=> {2} -> Unsupported endpoint pair")]
    InvalEp(String, String, String),
    #[error("{0}::{1} <=> {2}::{3} -> At least one endpoint is already bound")]
    AlrdyBind(String, bool, String, bool),
}

/// Port could be binded together
/// Port is here to hide the underlying packet type during the elaboration phase.
/// It enable to be generic on interface during the elaboration phase while keeping fully typed
/// communication at runtime
pub trait Port {
    /// Used to get ride of mutex and access the underlying endpoints
    fn view_as_handle(&self) -> PortHandle<'_>;

    /// Used during the elaboration phase to connect ports together
    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error>;
}

impl<T: Port> Port for std::sync::Arc<T> {
    fn view_as_handle(&self) -> PortHandle<'_> {
        (**self).view_as_handle()
    }

    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error> {
        (**self).bind(with_handle)
    }
}

/// Trait to genericly construct Port types in Port container
pub trait PortNew {
    fn new(
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: Option<usize>,
        sid_width: Option<usize>,
    ) -> Self;
}

/// Trait that provide a method to check communication status before sending over Port.
pub trait TxStatus {
    fn tx_check(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// Trait that provide a method to check communication status after receiving over Port.
pub trait RxStatus {
    fn rx_check(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// PortMutex
/// Endpoint locking are done by Hw process. Thus on acquire failure the scheduler must be notify
/// correctly to prevent deadlock in the simulation.
/// Provide custom locking function that correctly handle communication with Hw Scheduler.
/// Rely internally on tokio async Mutex for correct locking and inter-task notification
#[derive(Debug)]
pub struct PortMutex<T: ?Sized>(AtomicUsize, Mutex<T>);
pub struct PortMutexGuard<'a, T: ?Sized>(&'a AtomicUsize, MutexGuard<'a, T>);

impl<T: ?Sized + Endpoint + 'static> PortMutex<T> {
    /// Creates a new Endpoint in an unlocked state ready for use.
    pub fn new(t: T) -> Self
    where
        T: Sized,
    {
        PortMutex(AtomicUsize::new(0), Mutex::new(t))
    }

    /// Locks this port, causing the current prc to unregistered from Hw scheduler
    /// and yield until the lock has been acquired.
    /// When the lock has been acquired, process registers back in the scheduler
    /// and function returns a [`MutexGuard`].
    pub async fn lock(&self) -> PortMutexGuard<'_, T> {
        match self.1.try_lock() {
            Ok(port) => {
                // Endpoint locked directly.
                log!(|port| Category::Port, Verbosity::Debug
                        => => "Success, will yield");
                tokio::task::yield_now().await;
                log!(|port| Category::Port, Verbosity::Debug
                        => => "back from yield");
                PortMutexGuard(&self.0, port)
            }
            Err(_) => {
                log!(Category::Port, Verbosity::Debug
                        => => "Already locked, lock caller in scheduler");
                // Underlying port is locked, unregistered our process from the Hw Scheduler
                // Then start a blocking acquire and rely on tokio runtime for woken-up
                // After port correctly lock, register prc back in scheduler
                scheduler::SchedulerAPI::lock_prc();
                self.0.fetch_add(1_usize, Ordering::SeqCst);
                let port = self.1.lock().await;
                scheduler::SchedulerAPI::unlock_prc();
                self.0.fetch_sub(1_usize, Ordering::SeqCst);

                log!(Category::Port, Verbosity::Debug
                        => port.name() => "Unlock, unlock caller in scheduler");
                PortMutexGuard(&self.0, port)
            }
        }
    }

    /// Attempts to acquire the lock, and returns [`TryLockError`] if the
    /// lock is currently held somewhere else.
    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        self.1.try_lock()
    }
}

impl<T: ?Sized> Deref for PortMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl<T: ?Sized> DerefMut for PortMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.1
    }
}

impl<T: ?Sized> Drop for PortMutexGuard<'_, T> {
    fn drop(&mut self) {
        let waiters = self.0.load(Ordering::SeqCst);
        if waiters > 0 {
            scheduler::SchedulerAPI::unlock_evt();
        }
    }
}

/// PortVec enable variable port binding on the same module boundary.
/// A upper-bound is defined on Endpoint vector size due to preallocation of slice
/// This reduce the complexity of lifetime management, no reallocation, thus the
/// given reference have the PortVec lifetime
#[derive(Getters)]
pub struct PortVec<P> {
    name: String,
    props: Arc<module::Properties>,
    pkt_inflight: Option<usize>,
    sid_width: Option<usize>,
    port_vec: GrowOnlyVec<P>,
}

#[allow(clippy::len_without_is_empty)]
impl<P> PortVec<P> {
    pub fn new(
        capacity: usize,
        name: &str,
        props: Arc<module::Properties>,
        pkt_inflight: Option<usize>,
        sid_width: Option<usize>,
    ) -> Self {
        PortVec {
            name: name.to_string(),
            props,
            pkt_inflight,
            sid_width,
            port_vec: GrowOnlyVec::new(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.port_vec.len()
    }

    pub fn as_slice(&self) -> &[P] {
        self.port_vec.as_slice()
    }
}

impl<P> Index<usize> for PortVec<P> {
    type Output = P;

    fn index(&self, idx: usize) -> &Self::Output {
        &self.as_slice()[idx]
    }
}

impl<P: PortNew> PortVec<P> {
    fn next_port(&self) -> Result<&P, anyhow::Error> {
        let name = format!("{}_{}", self.name, self.port_vec.len());
        let port = P::new(&name, self.props.clone(), self.pkt_inflight, self.sid_width);
        let idx = self.port_vec.inner_mut_push(port)?;
        Ok(&self.port_vec.as_slice()[idx])
    }
}

/// Endpoint implementation for PortVec
impl<P: Port + PortNew> Port for PortVec<P> {
    /// Check last entry status
    /// If already binded allocated new port and return it as PortHandle
    /// otherwise return the last entry as PortHandle
    /// TODO currently no check were done
    fn view_as_handle(&self) -> PortHandle<'_> {
        let port = self
            .next_port()
            .unwrap_or_else(|_| panic!("{}.{}", self.props.path(), self.name));
        port.view_as_handle()
    }

    /// TODO currently no check were done
    fn bind(&self, with_handle: PortHandle) -> Result<(), anyhow::Error> {
        let port = self.next_port()?;
        port.bind(with_handle)?;
        Ok(())
    }
}
