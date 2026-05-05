//! Simulation time representation
//!
//! Define simulation time representation and timing mode
//!
use super::*;

use crossbeam::atomic::AtomicCell;
use once_cell::sync::OnceCell;
use std::fmt;
use std::io::{Error, ErrorKind};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Synchronisation granularity used between HW process
#[derive(Debug, Clone, Copy)]
pub enum TimingMode {
    /// LooselyTimed
    /// Delay are accumulated and only awaited once per communication
    LT,
    /// ApproximatelyTimed
    /// Delay are awaited at multiple stage during communication
    AT,
}

/// TimingMode used from CLI, thus provide FromStr/Display implementation for convenience
impl FromStr for TimingMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "LT" | "lt" | "LooselyTimed" | "Loosely"  => { Ok(Self::LT) },
            "AT" | "at" | "ApproxTimed" | "Approx"   => { Ok(Self::AT) },
            _ => { Err(Error::new(ErrorKind::InvalidInput, "Invalid TimingMode, expected \"LooselyTimed, Loosely, LT, ApproxTimed, Approx, AT\""))}
        }
    }
}

impl fmt::Display for TimingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Dedicated type to handle the simulation type
/// ATM, it here to ease the future feature extension of simulation Tick
pub type Tick = usize;
// #[derive(Debug, PartialOrd, Ord, PartialEq, Eq, Clone, Copy)]
// #[repr(transparent)]
// pub struct Tick(usize);
// impl From<usize> for Tick {
//     fn from(val: usize) -> Self {
//         Tick(val)
//     }
// }

/// Time representation used by the simulation
pub struct TimeKeeper {
    /// Current simulation time expressed as Tick
    pub tick: AtomicUsize,
    /// Simulation time scale: Number of Tick per second
    timescale: AtomicCell<unit::Time>,
    /// Simulation Timing Mode
    timing_mode: AtomicCell<TimingMode>,
}

impl TimeKeeper {
    /// Reset `TimeKeeper` in the global once_cell. If no instance available create a new one
    ///
    /// `tick` Reset simulated time at the given tick
    /// `timescale` Used time resolution
    /// `timing_mode` Used simulation timing mode
    ///
    pub fn reset(tick: Tick, timescale: unit::Time, timing_mode: TimingMode) {
        // Populate the once_cell instance if needed
        // NB: Testing required spawning of multiple simulation sequentially
        // This construct enable multiple setup of the once_cell content without issues
        let tk = TIME_KEEPER.get_or_init(|| TimeKeeper {
            tick: AtomicUsize::new(0),
            timescale: AtomicCell::new(unit::Time::fs(1)),
            timing_mode: AtomicCell::new(TimingMode::LT),
        });
        // Init/Reset TimeKeeper values
        println!("Reset TimeKeeper @{tick}[{timescale:?}]::{timing_mode:?}");
        tk.tick.store(tick, Ordering::SeqCst);
        tk.timescale.store(timescale);
        tk.timing_mode.store(timing_mode);
    }

    /// Ease access of global instance
    fn get_time() -> &'static TimeKeeper {
        TIME_KEEPER
            .get()
            .expect("TimeKeeper is not initialized. Did you run kernel::init(...)?")
    }

    /// Utility function to update current simulation tick
    pub fn update_tick(next_tick: Tick) {
        let tk = TimeKeeper::get_time();
        tk.tick.store(next_tick, Ordering::SeqCst);
    }

    /// Utility function to retrieved current simulation tick
    pub fn cur_tick() -> Tick {
        let tk = TimeKeeper::get_time();
        tk.tick.load(Ordering::SeqCst)
    }

    /// Utility function to try to retrieved current simulation tick
    /// Robustify the API when the simulation isn't totally init
    /// Used by logger or other debug features
    pub fn try_cur_tick() -> Option<Tick> {
        TIME_KEEPER.get().map(|tk| tk.tick.load(Ordering::SeqCst))
    }

    /// Utility function to retrieved current simulation timescale
    pub fn timescale() -> unit::Time {
        let tk = TimeKeeper::get_time();
        tk.timescale.load()
    }

    /// Utility function to retrieved current simulation timing_mode
    pub fn timing_mode() -> TimingMode {
        let tk = TimeKeeper::get_time();
        tk.timing_mode.load()
    }
}

/// Simulation time is backed by a global variable wrapped in once_cell
pub static TIME_KEEPER: OnceCell<TimeKeeper> = OnceCell::new();
