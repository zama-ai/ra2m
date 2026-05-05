//! Define Handler structure that gather history information
//!
//! This information are dumped to disk for later analyses through the trace macro
//!

use crate::*;
use ra2m_sim_containers::prelude::*;
use types::Span;

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

/// Size of preallocated entry for History
const STATIC_TRACE_SIZE: usize = 32;

/// Store list of previous occurred handling events
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History<T = ()>(PreAllocatedStore<Handler<T>, STATIC_TRACE_SIZE>);

impl<T> History<T> {
    pub fn duration_tick(&self) -> time::Tick {
        let hist_slice = self.0.as_slice();
        if hist_slice.is_empty() {
            0
        } else {
            hist_slice[hist_slice.len() - 1].tick() - hist_slice[0].tick()
        }
    }

    pub fn duration(&self) -> unit::Time {
        unit::Time::from(self.duration_tick()).rescale()
    }

    /// Extract a list of span from History
    /// Warn: Targeted component (i.e. deepest one) only handle transaction once, thus we should
    pub fn spans(&self) -> HashMap<usize, Span<time::Tick>> {
        let mut spans = HashMap::new();

        let mut uid_tick = self
            .as_slice()
            .iter()
            .map(|x| (x.uid(), x.tick()))
            .peekable();

        while let Some((uid, tick)) = uid_tick.next() {
            spans
                .entry(uid)
                .and_modify(|x: &mut Span<time::Tick>| {
                    x.end = tick;
                })
                .or_insert(Span {
                    start: tick,
                    end: uid_tick.peek().unwrap_or(&(tick, Default::default())).1,
                });
        }

        spans
    }
}

impl<T> std::ops::Deref for History<T> {
    type Target = PreAllocatedStore<Handler<T>, STATIC_TRACE_SIZE>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> std::ops::DerefMut for History<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> Display for History<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Duration: {}", self.duration())?;
        write!(f, "[")?;
        for (uid, span) in self.spans().into_iter() {
            write!(f, "{uid}: {span}")?;
        }
        write!(f, "]")
    }
}

/// Define handler structure that store event history to be stored in trace point.
/// Wrap the handling information to enable future extension with custom Info structure without
/// change in the Module library
/// During the traced object lifetime, Handler value will be pushed in a PreAllocatedStore.
/// At the end, they should be pushed in the associated trace backend to be formatted and stored on
/// disk for later analyses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum Handler<T = ()> {
    #[default]
    Unset,
    Base {
        tick: time::Tick,
        uid: usize,
    },
    Pipe {
        tick: time::Tick,
        uid: usize,
        pid: usize,
        status: PipeStatus,
    },
    Switch {
        tick: time::Tick,
        uid: usize,
        path: SwitchPath,
    },
    // Enable user to extend handler with custom properties
    Custom {
        tick: time::Tick,
        uid: usize,
        data: T,
    },
}

impl<T> Handler<T> {
    pub fn base(uid: usize) -> Self {
        Self::Base {
            tick: cur_tick(),
            uid,
        }
    }
    pub fn custom(uid: usize, data: T) -> Self {
        Self::Custom {
            tick: cur_tick(),
            uid,
            data,
        }
    }
    pub fn pipe(uid: usize, pid: usize, status: PipeStatus) -> Self {
        Self::Pipe {
            tick: cur_tick(),
            uid,
            pid,
            status,
        }
    }
    pub fn switch(uid: usize, path: SwitchPath) -> Self {
        Self::Switch {
            tick: cur_tick(),
            uid,
            path,
        }
    }
}

impl<T> Handler<T> {
    pub fn tick(&self) -> time::Tick {
        match *self {
            Self::Base { tick, .. } => tick,
            Self::Pipe { tick, .. } => tick,
            Self::Switch { tick, .. } => tick,
            Self::Custom { tick, .. } => tick,
            Self::Unset => panic!("Retrieved properties of unset handler"),
        }
    }
    pub fn uid(&self) -> usize {
        match *self {
            Self::Base { uid, .. } => uid,
            Self::Pipe { uid, .. } => uid,
            Self::Switch { uid, .. } => uid,
            Self::Custom { uid, .. } => uid,
            Self::Unset => panic!("Retrieved properties of unset handler"),
        }
    }
}

/// Used by component that switch traffic over multiple Path
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SwitchPath {
    Forward(usize, usize),  // Src -> Dst
    Backward(usize, usize), // Dst <- Src
}

/// Used by component that handle multi-stages computation
/// Depicts the overall computation progress
/// TODO: extend with other useful information
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PipeStatus(f64);

impl PipeStatus {
    pub fn from_qotient(cur_stage: usize, stages: usize) -> Self {
        assert!(
            cur_stage <= stages,
            "Quotient used for Pipeline progress status must be <= 1."
        );
        Self(100. * (cur_stage as f64 / stages as f64))
    }

    pub fn from_float(value: f64) -> Self {
        assert!(
            (0. ..=100.).contains(&value),
            "Pipeline progress status value must be between 0.0 and 100.0 (i.e. expressed in %)"
        );
        Self(value)
    }
}

impl Display for PipeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "progress: {}%", self.0)
    }
}

impl From<PipeStatus> for f64 {
    fn from(status: PipeStatus) -> f64 {
        status.0
    }
}
