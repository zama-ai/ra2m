//! Dma protocol
//! This is a address based protocol that enable deferred Read/Write transfer across two
//! addressable space.
//!

use std::sync::Arc;

use super::*;
use crate::port::{Packet, PacketOptions, RxStatus, TxStatus};
use crate::protocol::{addr::Pattern, Mode};
use crate::types::History;

use getset::{Getters, MutGetters, Setters};
use ra2m_sim_macros::Trace;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default DmaBusError type
/// Error status is encoded in the Mode fielf of Dma. To enable extensions from user side, it
/// rely on anyhow::Error.
#[derive(Error, Debug, Clone)]
pub enum DmaBusError {
    #[error("Request return as Response")]
    ReqAsResp,
    #[error("Response send as Request")]
    RespAsReq,
    #[error("Inner error")]
    Inner(Arc<anyhow::Error>),
}

/// Structure that define the DmaBus protocol
/// Contains request command with from/to addr and access pattern
///
/// No data handling is made, indeed those command are convert in Membus request for
/// execution
#[derive(Debug, Getters, MutGetters, Setters, Serialize, Deserialize, Trace)]
#[history(trace)]
#[getset(get = "pub", get_mut = "pub", set = "pub")]
pub struct DmaBus<T>
where
    T: Clone,
    for<'a> &'a T: Into<Traceable>,
{
    #[serde(skip)]
    mode: Mode<DmaBusError>,
    #[trace]
    read_from: T,
    #[trace]
    write_to: T,
    #[trace]
    pattern: Pattern,

    /// Contain history of the handling information of a given access through its route across the
    /// architecture (From the requester up to the responder and back for acknowledgement)
    trace: History,
}

impl<T> DmaBus<T>
where
    T: Clone + serde::Serialize,
    for<'a> &'a T: Into<Traceable>,
{
    pub fn new(read_from: T, write_to: T, pattern: Pattern) -> Self {
        Self {
            mode: Default::default(),
            read_from,
            write_to,
            pattern,
            trace: Default::default(),
        }
    }

    pub fn new_wrapped(
        read_from: T,
        write_to: T,
        pattern: Pattern,
        packet_options: Option<PacketOptions>,
    ) -> Packet<Self> {
        let options = packet_options.unwrap_or_default();
        Packet::wrap_payload(Self::new(read_from, write_to, pattern), options)
    }
}

impl<T> RxStatus for DmaBus<T>
where
    T: Clone + serde::Serialize,
    for<'a> &'a T: Into<Traceable>,
{
    fn rx_check(&self) -> Result<(), anyhow::Error> {
        match self.mode() {
            Mode::Request => Err(DmaBusError::ReqAsResp.into()),
            Mode::Response => Ok(()),
            Mode::Error(err) => Err(err.clone().into()),
        }
    }
}
impl<T> TxStatus for DmaBus<T>
where
    T: Clone + serde::Serialize,
    for<'a> &'a T: Into<Traceable>,
{
    fn tx_check(&self) -> Result<(), anyhow::Error> {
        match self.mode() {
            Mode::Response => Err(DmaBusError::RespAsReq.into()),
            Mode::Request => Ok(()),
            Mode::Error(err) => Err(err.clone().into()),
        }
    }
}
