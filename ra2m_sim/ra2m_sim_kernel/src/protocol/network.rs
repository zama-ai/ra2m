//! Network based protocol
//! Network abstraction could be used to modelize various OSI layer (i.e. MAC, IP, etc)
//!
//! It's used to wrap other protocol inside network one

use std::sync::Arc;

use super::*;
use crate::port::{Packet, PacketOptions, RxStatus, TxStatus};
use crate::protocol::Mode;
use crate::types::History;

use getset::{Getters, MutGetters, Setters};
use ra2m_sim_macros::Trace;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default NetworkError type
/// Error status is encoded in the Mode fielf of Network object. To enable extensions from user side, it
/// rely on anyhow::Error.
#[derive(Error, Debug, Clone)]
pub enum NetworkError {
    #[error("Request return as Response")]
    ReqAsResp,
    #[error("Response send as Request")]
    RespAsReq,
    #[error("Unreachable target")]
    Unreachable,
    #[error("Already used entry, check your binding")]
    AlreadyUsed,
    #[error("Inner error")]
    Inner(Arc<anyhow::Error>),
}

/// Structure that define the Packet protocol
/// Wrap any kind of data with a header
/// Contains request command with from/to addr and access pattern
#[derive(Debug, Getters, MutGetters, Setters, Serialize, Deserialize, Trace)]
#[history(trace)]
#[getset(get = "pub", get_mut = "pub", set = "pub")]
pub struct Network<T, P>
where
    T: Clone,
    for<'a> &'a T: Into<Traceable>,
{
    #[serde(skip)]
    mode: Mode<NetworkError>,
    #[trace]
    from: T,
    #[trace]
    to: T,
    payload: P,

    /// Contain history of the handling information of a given access through its route across the
    /// architecture (From the requester up to the responder and back for acknowledgement)
    trace: History,
}

impl<T, P> Network<T, P>
where
    T: Clone,
    for<'a> &'a T: Into<Traceable>,
{
    pub fn new(from: T, to: T, payload: P) -> Self {
        Self {
            mode: Default::default(),
            from,
            to,
            payload,
            trace: Default::default(),
        }
    }

    pub fn new_wrapped(
        from: T,
        to: T,
        payload: P,
        packet_options: Option<PacketOptions>,
    ) -> Packet<Self> {
        let options = packet_options.unwrap_or(Default::default());
        Packet::wrap_payload(Self::new(from, to, payload), options)
    }
}

impl<T, P> Packet<Network<T, P>>
where
    T: Clone,
    for<'a> &'a T: Into<Traceable>,
{
    /// Downcast to inner protocol
    /// Used to convert back into inner protocol while keeping unpaid delay
    pub fn inner_unwrap(self) -> Packet<P> {
        // Extract current options
        let timed = self.timed();
        let sid = self.sid().clone();
        let delay = self.delay();

        let Network { payload, .. } = self.unwrap_payload();
        Packet::wrap_payload(payload, PacketOptions { timed, sid, delay })
    }

    /// Wrap into network protocol
    /// Used to convert back into inner protocol while keeping unpaid delay
    pub fn inner_wrap(from: T, to: T, inner: Packet<P>) -> Self {
        // Extract current options
        let timed = inner.timed();
        let sid = inner.sid().clone();
        let delay = inner.delay();

        let payload = inner.unwrap_payload();
        Self::wrap_payload(
            Network::new(from, to, payload),
            PacketOptions { timed, sid, delay },
        )
    }
}

impl<T, P> RxStatus for Network<T, P>
where
    T: Clone,
    for<'a> &'a T: Into<Traceable>,
{
    fn rx_check(&self) -> Result<(), anyhow::Error> {
        match self.mode() {
            Mode::Request => Err(NetworkError::ReqAsResp.into()),
            Mode::Response => Ok(()),
            Mode::Error(err) => Err(err.clone().into()),
        }
    }
}
impl<T, P> TxStatus for Network<T, P>
where
    T: Clone,
    for<'a> &'a T: Into<Traceable>,
{
    fn tx_check(&self) -> Result<(), anyhow::Error> {
        match self.mode() {
            Mode::Response => Err(NetworkError::RespAsReq.into()),
            Mode::Request => Ok(()),
            Mode::Error(err) => Err(err.clone().into()),
        }
    }
}
