//! Implement various protocol on top of full_duplex port
//! Describe protocol based on data exchange over full_duplex port (e.g. ReqRespPort/DispatchPort)
//!
//! # MemBus protocol
//!   This is an address based protocol that enable to Read/Write data from an addressable space.
//!   Address could be expressed in the Physical/Virtual or both space.
//!   The communication size could be contiguous or encoded an stride pattern.
//!   Memory access history is stored in a the Trace entry and contain the list of access handler
//!   with the access status and the handling Tick.
//!
//! # Dma protocol
//!   This is a address based protocol that enable deferred Read/Write transfer across two
//!   addressable space.
//!   Reuse as much as possible from MemBus protocol for command definition
//!
use super::*;
use ra2m_sim_containers::prelude::*;
use types::{Handler, History};

/// Describe the current bus access status:
///  * Request: Access in on the fetch path
///  * Response: The target have handled the access, and it will acknowledge the requester
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum Mode<Err: std::error::Error> {
    #[default]
    Request,
    Response,
    Error(Err),
}
pub mod addr;

pub mod dma;
pub mod membus;
pub mod network;

/// Define trait for traceable object
/// It's used within protocol to kept track of object lifetime
/// Also used later for post-mortem analysis
///
/// A Traceable object have a dedicated store for handler history and provide two methods:
///  * One to extend the history with a given handler
///  * One to finish() the history stream with a Base Handler (done in trace! macro)
///  TODO: Should expose the Custom handler type as parameters
///
/// Aims of this traits is also to give hints for formatting into a DataFrame,
///  to prevent boilerplate code a derive macro is available.
/// It doesn't use polars::DataFrame directly to prevent deps (only load deps in the parser that really used it)
pub trait Trace: Sized + serde::Serialize {
    type Custom;

    /// Get reference to underlying History store
    fn get_history(&self) -> &History<Self::Custom>;

    /// Get mutable reference to underlying History store
    fn get_history_mut(&mut self) -> &mut History<Self::Custom>;

    /// Append given handler to history
    fn append_handler(&mut self, handler: Handler<Self::Custom>);

    /// Append a default wrap-up Handler
    /// This function aims to be used in output crate to automatically insert last module
    /// (i.e. the one that write the history trace point) as handler
    fn wrap_up(&mut self, uid: usize);

    fn export_as_traceable_map(
        vec: &Vec<Self>,
    ) -> std::collections::HashMap<&'static str, Vec<Traceable>>;
}

#[derive(Debug, Clone)]
pub enum Traceable {
    Text(String),
    Scalar(u64),
    Array(Vec<u64>),
}

#[macro_export]
macro_rules! traceable {
    ($native_type: ty, Traceable::Text) => {
        impl From<$native_type> for Traceable {
            fn from(value: $native_type) -> Self {
                Traceable::Text(value.into())
            }
        }

        impl From<&$native_type> for Traceable {
            fn from(value: &$native_type) -> Self {
                Traceable::Text(value.into())
            }
        }
    };
    ($native_type: ty, $kind: expr) => {
        impl From<$native_type> for Traceable {
            fn from(value: $native_type) -> Self {
                $kind(value as u64)
            }
        }

        impl From<&$native_type> for Traceable {
            fn from(value: &$native_type) -> Self {
                $kind(value.clone() as u64)
            }
        }
    };
}

traceable!(usize, Traceable::Scalar);
traceable!(u64, Traceable::Scalar);
traceable!(u32, Traceable::Scalar);
traceable!(u16, Traceable::Scalar);
traceable!(u8, Traceable::Scalar);
traceable!(String, Traceable::Text);
