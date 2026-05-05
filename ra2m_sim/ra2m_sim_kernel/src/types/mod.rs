//! Custom Simulation Type to describe HW properties
//!
//! This module implement type used in HW simulation alongside a set of commidity
//! function for conversion and inter types operations
//!
//! Defined types provide serde with clear user representation in Ron
//!

/// Types describing clock properties
pub mod clock;
pub use clock::{ClockDomain, Cycles, CyclesType};

/// Types describing history properties
pub mod history;
pub use history::{Handler, History, PipeStatus, SwitchPath};

/// Types describing latency properties
pub mod latency;
pub use latency::Latency;

/// Types describing periods of time
pub mod span;
pub use span::Span;

// Currently disabled not finish yet
// /// Types used to described fixed length integer
// pub mod uint;
