//! Address utilities
//!
//! Define everything related to address based access:
//!  * Addr with phys/virt support
//!  * Access pattern
//!  * AddrRange
//!  * Error associated with AddrRange access.
//!  * Routing table implementation that mapped AddrRange with associated entry
//!    Provide insertion facility with overlapping check and search function to find the associated
//!    range for a given access.
use super::*;
use crate::unit::DataUnit;

use std::collections::BTreeMap;
use thiserror::Error;

/// Addr representation in available address space.
/// Could be encoded as
///  * Physical address
///  * Virtual address (behind MMU)
///  * Or both (eg. an access that cross space boundary)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Addr {
    Unset(),
    Phys(usize),
    Virt(usize),
    PhysVirt(usize, usize),
    Range(usize, unit::Data),
}
impl Default for Addr {
    fn default() -> Self {
        Addr::Unset()
    }
}

impl Addr {
    pub fn as_phys(&self) -> Option<usize> {
        match self {
            Self::Phys(p) => Some(*p),
            Self::PhysVirt(p, _) => Some(*p),
            _ => None,
        }
    }

    pub fn as_virt(&self) -> Option<usize> {
        match self {
            Self::Virt(v) => Some(*v),
            Self::PhysVirt(_, v) => Some(*v),
            _ => None,
        }
    }

    pub fn with_offset(&self, offset: usize) -> Self {
        match *self {
            Self::Phys(p) => Self::Phys(p + offset),
            Self::Virt(v) => Self::Virt(v + offset),
            Self::PhysVirt(p, v) => Self::PhysVirt(p + offset, v + offset),
            Self::Range(b, range) => Self::Range(b + offset, range),
            _ => Self::Unset(),
        }
    }
}

impl From<&Addr> for Traceable {
    fn from(value: &Addr) -> Self {
        match value {
            Addr::Unset() => Traceable::Scalar(0),
            Addr::Phys(phys) => Traceable::Scalar(*phys as u64),
            Addr::Virt(virt) => Traceable::Scalar(*virt as u64),
            Addr::PhysVirt(phys, virt) => Traceable::Array(vec![*phys as u64, *virt as u64]),
            Addr::Range(addr, size) => {
                Traceable::Array(vec![*addr as u64, usize::from(size) as u64])
            }
        }
    }
}
//NB: Addr usually bound with index value for multi-node approach
// Provide Traceable cast for help
impl<T: Copy> From<&(T, Addr)> for Traceable
where
    u64: From<T>,
{
    fn from(value: &(T, Addr)) -> Self {
        let mh_dma_addr_trace = {
            let mut vec_trace = vec![u64::from(value.0)];
            match Traceable::from(&value.1) {
                Traceable::Scalar(val) => vec_trace.push(val),
                Traceable::Array(items) => vec_trace.extend_from_slice(items.as_slice()),
                _ => panic!("Unexpected Traceable type for Addr"),
            }
            vec_trace
        };
        Traceable::Array(mh_dma_addr_trace)
    }
}

// TODO implement Add/Sub on Addr

/// Addr representation in subrange
/// Addr expressed in subrange only support physical addr representation
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SubRangeAddr {
    Unset(),
    Phys(usize),
}
impl Default for SubRangeAddr {
    fn default() -> Self {
        SubRangeAddr::Unset()
    }
}
impl From<&SubRangeAddr> for Traceable {
    fn from(value: &SubRangeAddr) -> Self {
        match value {
            SubRangeAddr::Unset() => Traceable::Scalar(0),
            SubRangeAddr::Phys(phys) => Traceable::Scalar(*phys as u64),
        }
    }
}

/// Access pattern encoding:
///  * Simple: access on contiguous chunk of memory
///  * Stride: access on non-contiguous chunk of memory. Encoded as chunk size, stride offset and
///  required repetition.
///  NB: Stride pattern should be refine to match AHB/AXI4 specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pattern {
    Unset(),
    Simple(unit::Data),
    Stride(unit::Data, unit::Data, usize), // size, stride, iter
}
impl Pattern {
    /// Give the size of the access
    pub fn len(&self) -> unit::Data {
        match self {
            Self::Simple(d) => *d,
            Self::Stride(size, _, iter) => *size * *iter,
            _ => {
                panic!("Unsupported pattern {self:?}");
            }
        }
    }

    /// Give the range viewed by this access
    /// For contiguous access this value is equal to len
    pub fn range(&self) -> unit::Data {
        match self {
            Self::Simple(d) => *d,
            Self::Stride(_, stride, iter) => *stride * *iter,
            _ => {
                panic!("Unsupported pattern {self:?}");
            }
        }
    }
}
impl Default for Pattern {
    fn default() -> Self {
        Pattern::Unset()
    }
}

impl From<&Pattern> for Traceable {
    fn from(value: &Pattern) -> Self {
        match value {
            Pattern::Unset() => Traceable::Scalar(0),
            Pattern::Simple(size) => Traceable::Scalar(usize::from(size) as u64),
            Pattern::Stride(size, stride, iter) => Traceable::Array(vec![
                usize::from(size) as u64,
                usize::from(stride) as u64,
                *iter as u64,
            ]),
        }
    }
}

/// Generic error type associated with AddrRange access.
#[derive(Error, Clone, Debug, PartialEq, Eq)]
pub enum AddrRangeError {
    #[error("OverlapError: target AddrRange [0x{:x}:{:?}] overlaps with [0x{:x}:{:?}]",
            .target.0, .target.1, .overlap.0, .overlap.1)]
    Overlap {
        target: (usize, unit::Data),
        overlap: (usize, unit::Data),
    },
    #[error("CrossRangeError: Access [0x{:x}:{:?}] didn't fit in range map {range:?}",
            .access.0, .access.1)]
    CrossRange {
        access: (usize, unit::Data),
        range: BTreeMap<usize, (usize, usize)>,
    },
    #[error("UnmappedError:  AddrRange [0x{:x}:{:?}] is not mapped", .addr, .len)]
    Unmapped { addr: usize, len: unit::Data },
}

/// AddrRangeMap
/// Provide function to insert AddrRange in the map with overlapping check and a function to
/// retrieved the associated entry for a given AddrRange
#[derive(Debug)]
pub struct AddrRangeMap {
    /// Routing table as K:range_offset, V: (range_size, port_id)
    ar_map: BTreeMap<usize, (usize, usize)>,
}

impl AddrRangeMap {
    pub fn new() -> Self {
        AddrRangeMap {
            ar_map: BTreeMap::new(),
        }
    }

    pub fn insert_range(
        &mut self,
        addr_offset: usize,
        size: unit::Data,
        port_id: usize,
    ) -> Result<(), AddrRangeError> {
        // Check for overlapping
        if let Some((ar_ofst, (ar_size, _))) = self.ar_map.range(..=addr_offset).last() {
            if (ar_ofst + ar_size) > addr_offset {
                return Err(AddrRangeError::Overlap {
                    target: (addr_offset, size),
                    overlap: (*ar_ofst, ar_size.Byte()),
                });
            }
        }
        self.ar_map
            .insert(addr_offset, (usize::from(size), port_id));
        Ok(())
    }

    pub fn find_port(
        &self,
        addr_offset: usize,
        size: unit::Data,
    ) -> Result<(usize, usize), AddrRangeError> {
        // BTreeMap is ordered, finding the matching range if any is simply looking at the last
        // entry of all range with lower addr_offset
        if let Some((ar_ofst, (ar_size, port_id))) = self.ar_map.range(..=addr_offset).last() {
            if (ar_ofst + ar_size) < (addr_offset + usize::from(size)) {
                Err(AddrRangeError::CrossRange {
                    access: (addr_offset, size),
                    range: self.ar_map.clone(),
                })
            } else {
                Ok((*ar_ofst, *port_id))
            }
        } else {
            Err(AddrRangeError::Unmapped {
                addr: addr_offset,
                len: size,
            })
        }
    }
}

impl Default for AddrRangeMap {
    fn default() -> Self {
        Self::new()
    }
}
