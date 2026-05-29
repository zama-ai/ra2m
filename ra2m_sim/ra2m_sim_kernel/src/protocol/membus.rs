//! MemBus protocol
//! This is an address based protocol that enable to Read/Write data from an addressable space.
//! Address could be expressed in the Physical/Virtual or both space.
//! The communication size could be contiguous or encoded an stride pattern.
//! Memory access history is stored in a the Trace entry and contain the list of access handler
//! with the access status and the handling Tick.
//!

use super::*;
use crate::module::Properties;
use crate::port::{Packet, PacketOptions, ReqRespPort, RxStatus, TxStatus};
use crate::protocol::{
    addr::{Addr, AddrRangeError, Pattern, SubRangeAddr},
    Mode,
};
use crate::types::History;
use crate::unit::DataUnit;

use getset::{Getters, MutGetters, Setters};
use ra2m_sim_macros::Trace;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Access command
/// Encoded if the request is a Write or a Read
/// NB: This could be extend to describe other kind of access (eg. Atomic, Prefetch, ...)
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    #[default]
    Read,
    Write,
    AddrRange,
}

impl From<&Command> for Traceable {
    fn from(value: &Command) -> Self {
        match value {
            Command::Read => Traceable::Text("read".to_string()),
            Command::Write => Traceable::Text("write".to_string()),
            Command::AddrRange => Traceable::Text("addrrange".to_string()),
        }
    }
}

/// Default MemBusError type
/// Error status is encoded in the Mode fielf of MemBus. To enable extensions from user side, it
/// rely on anyhow::Error.
/// This is the default Membus implementation use to check that a request were correctly transform
/// in response
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MemBusError {
    #[error("Request return as Response")]
    ReqAsResp,
    #[error("Response send as Request")]
    RespAsReq,
    #[error("Command `{0:?}` unsupported by target")]
    Cmd(Command),
    #[error("Pattern `{0:?}` unsupported by target")]
    Pattern(Pattern),
    #[error("SubRange `{0:?}` unsupported by target")]
    SubRange(SubRangeAddr),
    #[error("AddrRangeMap: `{0:?}`")]
    AddrRange(AddrRangeError),
    #[error("Ffi: {0:?}")]
    Ffi(String),
}

const STATIC_DATA_SIZE: usize = 32; // Should match the cache-line in cpu-arch
const STATIC_XBAR_SIZE: usize = 16;
/// Request that define the Memory Bus protocol
/// Contains request command with addr/pattern, associated data and an handling history in the
/// trace field.
/// Handling history must be push to trace stream for post-processing and extraction of key metrics
/// of the simulated architecture.
#[derive(Debug, Getters, MutGetters, Setters, Serialize, Deserialize, Trace)]
#[history(trace)]
#[getset(get = "pub", get_mut = "pub", set = "pub")]
pub struct MemBus {
    #[serde(skip)]
    mode: Mode<MemBusError>,
    #[trace]
    cmd: Command,
    #[trace]
    addr: Addr,
    #[trace]
    subrange_addr: SubRangeAddr,
    #[trace]
    pattern: Pattern,
    data: PreAllocatedStore<u8, STATIC_DATA_SIZE>,

    /// Contain history of the handling information of a given access through its route across the
    /// architecture (From the requester up to the responder and back for acknowledgement)
    trace: History,

    /// Contain from_port history over xbar
    /// Enable to easily forward the acknowledgment back to the requester port
    /// It is used as a stack each Xbar push orig port and pop last entry when they forward the
    /// response back
    #[serde(skip)]
    xbar_port: PreAllocatedStore<usize, STATIC_XBAR_SIZE>,
}

impl MemBus {
    pub fn update_subrange(&mut self) {
        self.subrange_addr = match self.addr {
            Addr::Phys(paddr) => SubRangeAddr::Phys(paddr),
            Addr::PhysVirt(paddr, _vaddr) => SubRangeAddr::Phys(paddr),
            _ => SubRangeAddr::Unset(),
        };
    }

    pub fn new(
        from_uid: &usize,
        cmd: Command,
        addr: Addr,
        pattern: Pattern,
        data: Option<&[u8]>,
    ) -> Self {
        let mut membus = Self {
            mode: Default::default(),
            cmd,
            addr,
            subrange_addr: Default::default(),
            pattern,
            data: Default::default(),
            trace: Default::default(),
            xbar_port: Default::default(),
        };

        membus.update_subrange();

        if let Some(data) = data {
            membus.data_mut().extend_from_slice(data);
        }

        // Append issuer as first handler
        membus.wrap_up(*from_uid);
        membus
    }

    pub fn new_wrapped(
        from_uid: &usize,
        cmd: Command,
        addr: Addr,
        pattern: Pattern,
        data: Option<&[u8]>,
        packet_options: Option<PacketOptions>,
    ) -> Packet<Self> {
        let options = packet_options.unwrap_or_default();
        Packet::wrap_payload(Self::new(from_uid, cmd, addr, pattern, data), options)
    }
}

impl RxStatus for MemBus {
    fn rx_check(&self) -> Result<(), anyhow::Error> {
        match self.mode() {
            Mode::Request => Err(MemBusError::ReqAsResp.into()),
            Mode::Response => Ok(()),
            Mode::Error(err) => Err(err.clone().into()),
        }
    }
}
impl TxStatus for MemBus {
    fn tx_check(&self) -> Result<(), anyhow::Error> {
        match self.mode() {
            Mode::Response => Err(MemBusError::RespAsReq.into()),
            Mode::Request => Ok(()),
            Mode::Error(err) => Err(err.clone().into()),
        }
    }
}

/// MemBus is based on AddrRange
/// This function enable the proper addr_range registration at begin of the simulation. It must be
/// used by any module that use MemBus protocol.
pub async fn addr_range_register(
    from_uid: &usize,
    port: &ReqRespPort<MemBus>,
    ranges: &Vec<Addr>,
) -> Result<(), anyhow::Error> {
    for range in ranges {
        // Check ranges content
        match range {
            Addr::Range(_, _) => {}
            _ => {
                panic!("Addr_range must be expressed as Addr::Range(offset, size)")
            }
        };

        // Setup addr_range request
        let pkt = MemBus::new_wrapped(
            from_uid,
            Command::AddrRange,
            *range,
            Pattern::Unset(),
            None,
            None,
        );
        port.tx().send_pkt(pkt).await?;
    }

    // Acknowledge end of addr_map request
    let pkt = MemBus::new_wrapped(
        from_uid,
        Command::AddrRange,
        Addr::Unset(),
        Pattern::Unset(),
        None,
        None,
    );
    port.tx().send_pkt(pkt).await?;
    Ok(())
}

/// Implement custom function over ReqRespPort<MemBus> for easy memory based access
impl ReqRespPort<MemBus> {
    pub async fn discard_addr_range(&self) {
        let mut rx = self.rx().0.lock().await;
        loop {
            let pkt = rx.recv().await;
            let req = pkt.payload();
            match req.cmd() {
                Command::AddrRange => {
                    match req.addr() {
                        Addr::Range(_addr, _size) => {
                            // Drop addr range request
                        }
                        _ => {
                            // End of AddrRange setup for this port
                            break;
                        }
                    }
                }
                _ => {
                    panic! {"{self:?}: MemoryBus over ReqRespPort must start by AddrRange sequence"}
                }
            };
        }
    }

    // Access functions
    pub async fn write_bytes(
        &self,
        props: &Properties,
        addr: usize,
        data: &[u8],
    ) -> Result<(), anyhow::Error> {
        let _pkt = self
            .b_req_resp(MemBus::new_wrapped(
                props.uid(),
                Command::Write,
                Addr::Phys(addr),
                Pattern::Simple(data.len().Byte()),
                Some(data),
                None,
            ))
            .await?;
        Ok(())
    }

    pub async fn read_bytes(
        &self,
        props: &Properties,
        addr: usize,
        len: usize,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let pkt = self
            .b_req_resp(MemBus::new_wrapped(
                props.uid(),
                Command::Read,
                Addr::Phys(addr),
                Pattern::Simple(len.Byte()),
                None,
                None,
            ))
            .await?;
        let MemBus { data, .. } = pkt.unwrap_payload();
        Ok(data.into_vec())
    }

    pub async fn write<T: bytemuck::Pod>(
        &self,
        props: &Properties,
        addr: usize,
        data: &T,
    ) -> Result<(), anyhow::Error> {
        //NB: Use bytemuck Pod here to properly reject composed type such as Vec<_>, String, etc
        // This enforce safetiness of the cast
        let sliced_data = unsafe {
            std::slice::from_raw_parts(data as *const T as *const u8, std::mem::size_of::<T>())
        };
        self.write_bytes(props, addr, sliced_data).await
    }

    pub async fn read<T: bytemuck::Pod>(
        &self,
        props: &Properties,
        addr: usize,
        data: &mut T,
    ) -> Result<(), anyhow::Error> {
        let data_vec = self
            .read_bytes(props, addr, std::mem::size_of_val(data))
            .await?;

        //NB: Use bytemuck Pod here to properly reject composed type such as Vec<_>, String, etc
        // This enforce safetiness of the cast
        let sliced_data = unsafe {
            std::slice::from_raw_parts_mut(data as *mut T as *mut u8, std::mem::size_of::<T>())
        };
        sliced_data.clone_from_slice(data_vec.as_slice());
        Ok(())
    }
}
