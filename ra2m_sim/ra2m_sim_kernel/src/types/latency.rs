//! Custom Simulation Type to describe HW latency properties
//! Could be expressed cycle based on time based
//!
use crate::time::Tick;
use crate::types::clock::{ClockDomain, Cycles};
use crate::unit::Time;
use rand::RngExt;

#[derive(Debug, Clone)]
pub enum Latency {
    Cycle(Cycles),
    Time { avg: Time, var: Time },
}

impl Latency {
    pub fn into_tick(&self, clk_domain: &ClockDomain) -> crate::time::Tick {
        match self {
            Self::Cycle(cycles) => clk_domain.into_tick(*cycles),
            Self::Time { avg, var } => {
                let lat_avg: Tick = avg.into();
                let lat_var: Tick = var.into();
                let mut rng = rand::rng();
                lat_avg + rng.random_range(0..lat_var)
            }
        }
    }
}
