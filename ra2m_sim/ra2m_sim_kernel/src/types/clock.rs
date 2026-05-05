//! Custom Simulation Type to describe HW clock properties
//!

use crate::unit::{Frequency, FrequencyUnit};

use getset::Getters;
use serde::{Deserialize, Serialize};

/// Cycles representation
/// Simple type backed by usize for representing cycle counts, i.e. a relative
/// difference between two points in time, expressed in a number of clock cycles.
///
/// Module trait provide conversion into Tick based on Module frequency
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Cycles(usize);

impl Cycles {}

/// Trait use for casting from primitive type with a clear syntax
pub trait CyclesType {
    fn cycles(&self) -> Cycles;
}

impl<T: num::ToPrimitive> CyclesType for T {
    fn cycles(&self) -> Cycles {
        Cycles(self.to_usize().unwrap())
    }
}

impl From<Cycles> for usize {
    fn from(cycles: Cycles) -> Self {
        cycles.0
    }
}

impl std::ops::Mul<usize> for Cycles {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        Self(self.0 * rhs)
    }
}
impl std::ops::Div<usize> for Cycles {
    type Output = Self;

    fn div(self, rhs: usize) -> Self {
        Self(self.0 / rhs)
    }
}
impl std::ops::Add for Cycles {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}
impl std::ops::Sub for Cycles {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

/// ClockDomain representation
///
/// The ClockDomain provides clock properties to group of Module bundled under
/// the same clock domain.
/// TODO: The clock domains provide support for a hierarchial structure with
/// source and derived domains.
///
#[derive(Clone, Copy, Debug, Getters)]
#[getset(get = "pub")]
pub struct ClockDomain {
    frequency: Frequency,
    // TODO extend with gated status and derived clock dependence
}

impl ClockDomain {
    pub fn new(frequency: Frequency) -> Self {
        Self { frequency }
    }

    pub fn into_tick(&self, cycles: Cycles) -> crate::time::Tick {
        let period: crate::unit::Time = self.frequency.into();
        // NB: in order to prevent arith error on back and forth conversion
        // Tick conversion for cycle is sampled 25% after clock edge
        ((period * cycles) + period / 4.0).into()
    }
    pub fn from_tick(&self, tick: crate::time::Tick) -> Cycles {
        let cur_time: crate::unit::Time = tick.into();
        let period: crate::unit::Time = self.frequency.into();
        // Natural flooring since fs is expressed as usize
        (cur_time.into_fs() / period.into_fs()).cycles()
    }
}
// TODO: update default value => derived from hierarchy instead of fixed 1GHz
// This 1GHz value was here to prevent lot of breaking in module interface, must be only temporary
// workaround.
impl Default for ClockDomain {
    fn default() -> Self {
        Self::new(1.GHz())
    }
}
