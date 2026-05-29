//! Packet are used to exchange payload across port
//!
//! A packet associated metadata to the payload to correctly handle data exchange over ports.
//! The metadata are used to handle the communication cost delay and implement packet reordering
//! and dispatch over sessions
use super::*;

use getset::{CopyGetters, Getters, MutGetters, Setters};
use std::sync::atomic::{AtomicUsize, Ordering};
use thiserror::Error;

/// Static atomic counter used to generate packet unique id (uid)
static PACKET_UID_GEN: AtomicUsize = AtomicUsize::new(0);

/// Abstract version of Packet
/// Wrapped a payload with communication properties to be sent across port.
#[derive(Debug, CopyGetters, Getters, MutGetters, Setters)]
pub struct Packet<T> {
    /// Enable full delay propagation for this packet
    /// By default this flag is turn on
    /// Could be disable for fast-path or debug introspection
    /// NB: delay is still computed but never awaited in `handle_delay`
    #[getset(get_copy = "pub", set = "pub")]
    timed: bool,

    /// Unique Id: used for response reordering
    #[getset(get_copy = "pub")]
    uid: usize,
    /// Session Id: used to easily handle multiple stream of packet through ports
    #[getset(get = "pub", set = "pub")]
    sid: Option<usize>,
    /// Underlying data
    #[getset(get = "pub", get_mut = "pub", set = "pub")]
    payload: T,
    /// The underlying data will be available after the given tick
    ready_at: time::Tick,
}

impl<T> Packet<T> {
    /// Wrap an existing payload in a new Packet
    pub fn wrap_payload(payload: T, options: PacketOptions) -> Self {
        Self {
            timed: options.timed,
            uid: PACKET_UID_GEN.fetch_add(1_usize, Ordering::SeqCst),
            sid: options.sid,
            payload,
            ready_at: time::TimeKeeper::cur_tick() + options.delay,
        }
    }

    /// Unwrap a Payload from an existing Packet
    pub fn unwrap_payload(self) -> T {
        let Self { payload, .. } = self;
        payload
    }
}

/// Various function used to handle packet delay
impl<T> Packet<T> {
    /// Handle communication delay
    /// => Await until ready tick of the packet
    pub async fn handle_delay(&mut self) {
        if self.timed && self.ready_at > time::TimeKeeper::cur_tick() {
            tokio::task::yield_now().await; // TODO understand resolved MT issue
            delay::Delay::wait_until(self.ready_at).await;
        }
    }

    /// Account for communication delay
    /// Behavior based on simulation timing mode:
    ///  * LooselyTimed (LT): delay appended to packet counter
    ///  * ApproximatelyTimed (AT): delay directly consumed
    pub async fn account_delay(&mut self, delay: Option<time::Tick>) {
        if let Some(delay) = delay {
            self.ready_at += delay;
        }
        match time::TimeKeeper::timing_mode() {
            time::TimingMode::LT => {} // Only Accumulate delay
            time::TimingMode::AT => {
                // Await delay
                self.handle_delay().await
            }
        }
    }

    /// Append delay
    /// Increase delay counter without waiting for it whatever the current timing mode.
    /// This is used to deferred timing handle to utilities function defined inside port
    ///  -> Packet delay is handle while port is locked
    pub fn append_delay(&mut self, delay: time::Tick) {
        self.ready_at += delay;
    }

    /// Get delay
    /// Convert inner ready_at attribute into real delay
    pub fn delay(&self) -> time::Tick {
        let cur_tick = time::TimeKeeper::cur_tick();

        self.ready_at.saturating_sub(cur_tick)
    }
}

impl<T> std::cmp::PartialOrd for Packet<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // Ordering based on uid only
        self.uid().partial_cmp(&other.uid())
        // TODO handle the case of uid wrapping
    }
}

impl<T> std::cmp::PartialEq for Packet<T> {
    fn eq(&self, other: &Self) -> bool {
        // Ordering based on uid only
        self.uid().eq(&other.uid())
    }
}

/// PacketOptions
/// Enable packet configuration during creation
#[derive(Debug, Clone, Copy)]
pub struct PacketOptions {
    pub timed: bool,
    pub sid: Option<usize>,
    pub delay: time::Tick,
}

impl Default for PacketOptions {
    fn default() -> Self {
        Self {
            timed: true,
            sid: Default::default(),
            delay: Default::default(),
        }
    }
}

/// Packet Error handling
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PacketError {
    #[error("Untagged port used as communication endpoint for packet with sid {0}")]
    UntaggedSid(usize),
    #[error("Tagged port used with packet without sid")]
    TaggedNoSid,
    #[error("Received packet uid don't match requested uid [received: {0}, requested: {1}]")]
    UidMismatch(usize, usize),
}
