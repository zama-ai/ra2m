//! Include memory models definition directly in the mem module
//!

pub mod dma;
pub use self::dma::{Dma, DmaParams};

pub mod np_ram;
pub use self::np_ram::{NpRam, NpRamParams};

pub mod xbar;
pub use self::xbar::{XBar, XBarParams};
