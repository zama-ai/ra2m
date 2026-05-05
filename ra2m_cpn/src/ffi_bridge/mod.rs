//! External interface
//!
//! Provides a set of communication between simulation and external components
//! Used to simulate Host <> Sim and Sim <-Host interaciton

/// IPC interfaces to enable Host/Sim communications
pub mod ipc;

#[cfg(feature = "rpc")]
/// RPC interfaces to enable Host/Sim communications
// Rely on gRPC and required some external tool.
// Hide bedind a feature flag to let it optional
pub mod rpc;
