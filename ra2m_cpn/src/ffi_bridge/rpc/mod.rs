//! RemotePort protocol
//!
//! Enable Host <-> Sim and Sim <-> Host communication through gRPC.
//! Define an protobuf format that enable MemBus like req/resp between Host and ra2m.
//! This abstraction enable to bridge ra2m simulated architecture with the host software stack.
//!

/// Host2Sim Bridge
/// Host send read/write request into ra2m simulated architecture
pub mod h2s_bridge;
pub use self::h2s_bridge::{H2sBridge, H2sBridgeParams};

/// Sim2Host Bridge
/// Host received read/write request from ra2m simulated architecture
/// Use to emulate DMA like behavior
pub mod s2h_bridge;
pub use self::s2h_bridge::{S2hBridge, S2hBridgeParams};
