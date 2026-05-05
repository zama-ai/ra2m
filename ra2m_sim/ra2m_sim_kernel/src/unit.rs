//! Unit system
//!
//! This module implement a fairly simple unit system that define units in used in HW simulation
//! alongside a set of commidity function for conversion and inter unit operations
//!
//! Defined unit provide serde with clear user representation in Ron
//!
use super::*;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Error, ErrorKind};
use std::str::FromStr;

/// Time representation
/// Lowest unit is the fento-second. All conversion and operation are done in this unit
/// From trait is implemented for Tick and simple ops are defined on Time
///
/// For unit != fs, the underlying type is f64.
/// NB: Scheduler only understand Tick, thus the underlying representation don't have direct impact
/// on performances
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[allow(non_camel_case_types)]
pub enum Time {
    fs(usize),
    ps(f64),
    ns(f64),
    us(f64),
    ms(f64),
    s(f64),
}
const UNIT_SCALE: [(Time, usize); 6] = [
    (Time::s(1.), 10_u64.pow(15) as usize),
    (Time::ms(1.), 10_u64.pow(12) as usize),
    (Time::us(1.), 10_u64.pow(9) as usize),
    (Time::ns(1.), 10_u64.pow(6) as usize),
    (Time::ps(1.), 10_u64.pow(3) as usize),
    (Time::fs(1), 1),
];

impl Time {
    pub fn into_fs(self) -> usize {
        match self {
            Time::fs(t) => t,
            Time::ps(t) => (10_u64.pow(3) as f64 * t).round() as usize,
            Time::ns(t) => (10_u64.pow(6) as f64 * t).round() as usize,
            Time::us(t) => (10_u64.pow(9) as f64 * t).round() as usize,
            Time::ms(t) => (10_u64.pow(12) as f64 * t).round() as usize,
            Time::s(t) => (10_u64.pow(15) as f64 * t).round() as usize,
        }
    }
    pub fn rescale(self) -> Self {
        let t_fs = self.into_fs();
        UNIT_SCALE
            .iter()
            .filter(|(_u, s)| t_fs >= *s)
            .map(|(u, s)| *u * (t_fs as f64 / *s as f64))
            .next()
            .unwrap_or(Self::fs(0))
    }
}

/// Time used from CLI, thus provide FromStr/Display implementation for convenience
impl FromStr for Time {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val_unit: Vec<&str> = s.split('_').collect();
        if val_unit.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid split, expected \"val_unit\"",
            ));
        }

        // Parse value
        let value;
        if let Ok(val) = val_unit[0].parse::<f64>() {
            value = val;
        } else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid value, expected \"(val as f64)_unit\"",
            ));
        }

        // Parse units
        match val_unit[1] {
            "fs" => Ok(value.fs()),
            "ps" => Ok(value.ps()),
            "ns" => Ok(value.ns()),
            "us" => Ok(value.us()),
            "ms" => Ok(value.ms()),
            "s" => Ok(value.s()),
            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid unit, expected \"val_[fs,ps,ns,us,ms,s]\"",
            )),
        }
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Time::fs(t) => write!(f, "{t}fs"),
            Time::ps(t) => write!(f, "{t}ps"),
            Time::ns(t) => write!(f, "{t}ns"),
            Time::us(t) => write!(f, "{t}us"),
            Time::ms(t) => write!(f, "{t}ms"),
            Time::s(t) => write!(f, "{t}s"),
        }
    }
}

/// Trait use for casting from primitive type with a clear syntax
pub trait TimeUnit {
    fn fs(&self) -> Time;
    fn ps(&self) -> Time;
    fn ns(&self) -> Time;
    fn us(&self) -> Time;
    fn ms(&self) -> Time;
    fn s(&self) -> Time;
}

impl<T: num::ToPrimitive> TimeUnit for T {
    fn fs(&self) -> Time {
        Time::fs(self.to_usize().unwrap())
    }
    fn ps(&self) -> Time {
        Time::ps(self.to_f64().unwrap())
    }
    fn ns(&self) -> Time {
        Time::ns(self.to_f64().unwrap())
    }
    fn us(&self) -> Time {
        Time::us(self.to_f64().unwrap())
    }
    fn ms(&self) -> Time {
        Time::ms(self.to_f64().unwrap())
    }
    fn s(&self) -> Time {
        Time::s(self.to_f64().unwrap())
    }
}

impl From<Time> for time::Tick {
    fn from(t: Time) -> Self {
        let ts = time::TimeKeeper::timescale();
        t.into_fs() / ts.into_fs()
    }
}

impl From<&Time> for time::Tick {
    fn from(t: &Time) -> Self {
        let ts = time::TimeKeeper::timescale();
        num::integer::div_ceil(t.into_fs(), ts.into_fs())
    }
}

impl From<time::Tick> for Time {
    fn from(tick: time::Tick) -> Self {
        let ts = time::TimeKeeper::timescale();
        ts * (tick as f64)
    }
}

impl std::ops::Mul<f64> for Time {
    type Output = Time;

    fn mul(self, rhs: f64) -> Self {
        match self {
            Self::fs(t_fs) => Self::fs(((t_fs as f64) * rhs).round() as usize),
            Self::ps(t_ps) => Self::ps(t_ps * rhs),
            Self::ns(t_ns) => Self::ns(t_ns * rhs),
            Self::us(t_us) => Self::us(t_us * rhs),
            Self::ms(t_ms) => Self::ms(t_ms * rhs),
            Self::s(t_s) => Self::s(t_s * rhs),
        }
    }
}
impl std::ops::Mul<types::clock::Cycles> for Time {
    type Output = Time;

    fn mul(self, rhs: types::clock::Cycles) -> Self {
        let rhs_raw = usize::from(rhs);
        match self {
            Self::fs(t_fs) => Self::fs(t_fs * rhs_raw),
            Self::ps(t_ps) => Self::ps(t_ps * rhs_raw as f64),
            Self::ns(t_ns) => Self::ns(t_ns * rhs_raw as f64),
            Self::us(t_us) => Self::us(t_us * rhs_raw as f64),
            Self::ms(t_ms) => Self::ms(t_ms * rhs_raw as f64),
            Self::s(t_s) => Self::s(t_s * rhs_raw as f64),
        }
    }
}

impl std::ops::Div<Self> for Time {
    type Output = Time;

    fn div(self, rhs: Self) -> Self {
        let div_fs = self.into_fs() / rhs.into_fs();
        Self::fs(div_fs)
    }
}

impl std::ops::Div<f64> for Time {
    type Output = Time;

    fn div(self, rhs: f64) -> Self {
        match self {
            Self::fs(t_fs) => Self::fs(((t_fs as f64) / rhs).round() as usize),
            Self::ps(t_ps) => Self::ps(t_ps / rhs),
            Self::ns(t_ns) => Self::ns(t_ns / rhs),
            Self::us(t_us) => Self::us(t_us / rhs),
            Self::ms(t_ms) => Self::ms(t_ms / rhs),
            Self::s(t_s) => Self::s(t_s / rhs),
        }
    }
}

impl std::ops::Add for Time {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        let lhs_fs = self.into_fs();
        let rhs_fs = rhs.into_fs();
        Self::fs(lhs_fs + rhs_fs)
    }
}
impl std::ops::Sub for Time {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        let lhs_fs = self.into_fs();
        let rhs_fs = rhs.into_fs();
        Self::fs(lhs_fs - rhs_fs)
    }
}

/// Frequency representation
/// Lowest unit is the Hertz. All conversion and operation are done in this unit
/// From trait is implemented for Time (T = 1/F) and simple ops are defined on Frequency
///
/// For unit != Hz, the underlying type is f64.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[allow(non_camel_case_types)]
pub enum Frequency {
    Hz(usize),
    kHz(f64),
    MHz(f64),
    GHz(f64),
    THz(f64),
}

impl Frequency {
    fn into_hz(self) -> usize {
        match self {
            Self::Hz(t) => t,
            Self::kHz(t) => (10_u64.pow(3) as f64 * t).round() as usize,
            Self::MHz(t) => (10_u64.pow(6) as f64 * t).round() as usize,
            Self::GHz(t) => (10_u64.pow(9) as f64 * t).round() as usize,
            Self::THz(t) => (10_u64.pow(12) as f64 * t).round() as usize,
        }
    }
}

/// Frequency used from CLI, thus provide FromStr/Display implementation for convenience
impl FromStr for Frequency {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val_unit: Vec<&str> = s.split('_').collect();
        if val_unit.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid split, expected \"val_unit\"",
            ));
        }

        // Parse value
        let value;
        if let Ok(val) = val_unit[0].parse::<f64>() {
            value = val;
        } else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid value, expected \"(val as f64)_unit\"",
            ));
        }

        // Parse units
        match val_unit[1].to_lowercase().as_str() {
            "Hz" => Ok(value.Hz()),
            "khz" => Ok(value.kHz()),
            "mhz" => Ok(value.MHz()),
            "ghz" => Ok(value.GHz()),
            "thz" => Ok(value.THz()),
            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid unit, expected \"val_[Hz, kHz, MHz, GHz, THz]\"",
            )),
        }
    }
}

impl fmt::Display for Frequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Trait use for casting from primitive type with a clear syntax
#[allow(non_snake_case)]
pub trait FrequencyUnit {
    fn Hz(&self) -> Frequency;
    fn kHz(&self) -> Frequency;
    fn MHz(&self) -> Frequency;
    fn GHz(&self) -> Frequency;
    fn THz(&self) -> Frequency;
}

#[allow(non_snake_case)]
impl<T: num::ToPrimitive> FrequencyUnit for T {
    fn Hz(&self) -> Frequency {
        Frequency::Hz(self.to_usize().unwrap())
    }
    fn kHz(&self) -> Frequency {
        Frequency::kHz(self.to_f64().unwrap())
    }
    fn MHz(&self) -> Frequency {
        Frequency::MHz(self.to_f64().unwrap())
    }
    fn GHz(&self) -> Frequency {
        Frequency::GHz(self.to_f64().unwrap())
    }
    fn THz(&self) -> Frequency {
        Frequency::THz(self.to_f64().unwrap())
    }
}

impl From<Frequency> for Time {
    fn from(f: Frequency) -> Self {
        let scale = 1.s().into_fs();
        Self::fs(scale / f.into_hz())
    }
}
impl From<&Frequency> for Time {
    fn from(f: &Frequency) -> Self {
        let scale = 1.s().into_fs();
        Self::fs(scale / f.into_hz())
    }
}

impl std::ops::Mul<f64> for Frequency {
    type Output = Frequency;

    fn mul(self, rhs: f64) -> Self {
        // TODO? conserve original unit ?
        let lhs_hz = self.into_hz();
        let mul_hz = (lhs_hz as f64 * rhs).round() as usize;
        Self::Hz(mul_hz)
    }
}
impl std::ops::Div<f64> for Frequency {
    type Output = Frequency;

    fn div(self, rhs: f64) -> Self {
        // TODO? conserve original unit ?
        let lhs_hz = self.into_hz();
        let div_hz = (lhs_hz as f64 / rhs).round() as usize;
        Self::Hz(div_hz)
    }
}
impl std::ops::Add for Frequency {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        let lhs_hz = self.into_hz();
        let rhs_hz = rhs.into_hz();
        Self::Hz(lhs_hz + rhs_hz)
    }
}
impl std::ops::Sub for Frequency {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        let lhs_hz = self.into_hz();
        let rhs_hz = rhs.into_hz();
        Self::Hz(lhs_hz - rhs_hz)
    }
}

/// Data representation
/// Lowest unit is the Byte. All conversion and operation are done in this unit
/// Simple ops are defined on Data
///
/// For unit != Byte, the underlying type is f64.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[allow(non_camel_case_types)]
pub enum Data {
    Byte(usize),
    kB(f64),
    KiB(f64),
    MB(f64),
    MiB(f64),
    GB(f64),
    GiB(f64),
    TB(f64),
    TiB(f64),
    PB(f64),
    PiB(f64),
    ZB(f64),
    ZiB(f64),
}

impl Data {
    fn into_byte(self) -> usize {
        match self {
            Self::Byte(b) => b,
            Self::kB(b) => (1000_f64 * b).round() as usize,
            Self::KiB(b) => (1024_f64 * b).round() as usize,
            Self::MB(b) => (1000_u64.pow(2) as f64 * b).round() as usize,
            Self::MiB(b) => (1024_u64.pow(2) as f64 * b).round() as usize,
            Self::GB(b) => (1000_u64.pow(3) as f64 * b).round() as usize,
            Self::GiB(b) => (1024_u64.pow(3) as f64 * b).round() as usize,
            Self::TB(b) => (1000_u64.pow(4) as f64 * b).round() as usize,
            Self::TiB(b) => (1024_u64.pow(4) as f64 * b).round() as usize,
            Self::PB(b) => (1000_u64.pow(5) as f64 * b).round() as usize,
            Self::PiB(b) => (1024_u64.pow(5) as f64 * b).round() as usize,
            Self::ZB(b) => (1000_u64.pow(6) as f64 * b).round() as usize,
            Self::ZiB(b) => (1024_u64.pow(6) as f64 * b).round() as usize,
        }
    }
}

/// Time used from CLI, thus provide FromStr/Display implementation for convenience
impl FromStr for Data {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val_unit: Vec<&str> = s.split('_').collect();
        if val_unit.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid split, expected \"val_unit\"",
            ));
        }

        // Parse value
        let value;
        if let Ok(val) = val_unit[0].parse::<f64>() {
            value = val;
        } else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid value, expected \"(val as f64)_unit\"",
            ));
        }

        // Parse units
        match val_unit[1] {
            "B" => Ok(value.Byte()),
            "kB" => Ok(value.kB()),
            "KiB" => Ok(value.KiB()),
            "MB" => Ok(value.MB()),
            "MiB" => Ok(value.MiB()),
            "GB" => Ok(value.GB()),
            "GiB" => Ok(value.GiB()),
            "TB" => Ok(value.TB()),
            "TiB" => Ok(value.TiB()),
            "PB" => Ok(value.PB()),
            "PiB" => Ok(value.PiB()),
            "ZB" => Ok(value.ZB()),
            "ZiB" => Ok(value.ZiB()),
            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid unit, expected \"val_[B, kB, KiB ... ZB, ZiB]\"",
            )),
        }
    }
}

impl fmt::Display for Data {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Trait use for casting from primitive type with a clear syntax
#[allow(non_snake_case)]
pub trait DataUnit {
    fn Byte(&self) -> Data;
    fn kB(&self) -> Data;
    fn KiB(&self) -> Data;
    fn MB(&self) -> Data;
    fn MiB(&self) -> Data;
    fn GB(&self) -> Data;
    fn GiB(&self) -> Data;
    fn TB(&self) -> Data;
    fn TiB(&self) -> Data;
    fn PB(&self) -> Data;
    fn PiB(&self) -> Data;
    fn ZB(&self) -> Data;
    fn ZiB(&self) -> Data;
}

#[allow(non_snake_case)]
impl<T: num::ToPrimitive> DataUnit for T {
    fn Byte(&self) -> Data {
        Data::Byte(self.to_usize().unwrap())
    }
    fn kB(&self) -> Data {
        Data::kB(self.to_f64().unwrap())
    }
    fn KiB(&self) -> Data {
        Data::KiB(self.to_f64().unwrap())
    }
    fn MB(&self) -> Data {
        Data::MB(self.to_f64().unwrap())
    }
    fn MiB(&self) -> Data {
        Data::MiB(self.to_f64().unwrap())
    }
    fn GB(&self) -> Data {
        Data::GB(self.to_f64().unwrap())
    }
    fn GiB(&self) -> Data {
        Data::GiB(self.to_f64().unwrap())
    }
    fn TB(&self) -> Data {
        Data::TB(self.to_f64().unwrap())
    }
    fn TiB(&self) -> Data {
        Data::TiB(self.to_f64().unwrap())
    }
    fn PB(&self) -> Data {
        Data::PB(self.to_f64().unwrap())
    }
    fn PiB(&self) -> Data {
        Data::PiB(self.to_f64().unwrap())
    }
    fn ZB(&self) -> Data {
        Data::ZB(self.to_f64().unwrap())
    }
    fn ZiB(&self) -> Data {
        Data::ZiB(self.to_f64().unwrap())
    }
}

impl From<Data> for usize {
    fn from(d: Data) -> Self {
        d.into_byte()
    }
}
impl From<&Data> for usize {
    fn from(d: &Data) -> Self {
        d.into_byte()
    }
}
impl PartialEq for Data {
    fn eq(&self, other: &Self) -> bool {
        self.into_byte() == other.into_byte()
    }
}
impl Eq for Data {}

/// Data is considered as dimentionless => [Data]*[Data] = [Data]
impl std::ops::Mul<f64> for Data {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self {
        // TODO? conserve original unit ?
        let lhs_b = self.into_byte();
        let mul_b = (lhs_b as f64 * rhs).round() as usize;
        Self::Byte(mul_b)
    }
}
impl std::ops::Div<f64> for Data {
    type Output = Self;

    fn div(self, rhs: f64) -> Self {
        // TODO? conserve original unit ?
        let lhs_b = self.into_byte();
        let div_b = (lhs_b as f64 / rhs).round() as usize;
        Self::Byte(div_b)
    }
}
impl std::ops::Mul<usize> for Data {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        // TODO? conserve original unit ?
        let lhs_b = self.into_byte();
        Self::Byte(lhs_b * rhs)
    }
}
impl std::ops::Div<usize> for Data {
    type Output = Self;

    fn div(self, rhs: usize) -> Self {
        // TODO? conserve original unit ?
        let lhs_b = self.into_byte();
        Self::Byte(lhs_b / rhs)
    }
}
impl std::ops::Div<BW> for Data {
    type Output = Time;

    fn div(self, rhs: BW) -> Self::Output {
        let lhs_b = self.into_byte();
        let rhs_b_s = rhs.into_byte_s();
        let scale_s_as_fs: usize = 1.s().into();
        Self::Output::fs(((scale_s_as_fs as u128 * lhs_b as u128) / rhs_b_s as u128) as usize)
    }
}
impl std::ops::Add for Data {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        let lhs_b = self.into_byte();
        let rhs_b = rhs.into_byte();
        Self::Byte(lhs_b + rhs_b)
    }
}
impl std::ops::Sub for Data {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        let lhs_b = self.into_byte();
        let rhs_b = rhs.into_byte();
        Self::Byte(lhs_b - rhs_b)
    }
}

/// BandWidth (BW) representation
/// Lowest unit is the Byte_s. All conversion and operation are done in this unit
/// Simple ops are defined on BandWidth
///
/// For unit != Byte_s, the underlying type is f64.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[allow(non_camel_case_types)]
pub enum BW {
    Byte_s(usize),
    kB_s(f64),
    KiB_s(f64),
    MB_s(f64),
    MiB_s(f64),
    GB_s(f64),
    GiB_s(f64),
    TB_s(f64),
    TiB_s(f64),
    PB_s(f64),
    PiB_s(f64),
    ZB_s(f64),
    ZiB_s(f64),
}

impl BW {
    fn into_byte_s(self) -> usize {
        match self {
            Self::Byte_s(b) => b,
            Self::kB_s(b) => (1000_f64 * b).round() as usize,
            Self::KiB_s(b) => (1024_f64 * b).round() as usize,
            Self::MB_s(b) => (1000_u64.pow(2) as f64 * b).round() as usize,
            Self::MiB_s(b) => (1024_u64.pow(2) as f64 * b).round() as usize,
            Self::GB_s(b) => (1000_u64.pow(3) as f64 * b).round() as usize,
            Self::GiB_s(b) => (1024_u64.pow(3) as f64 * b).round() as usize,
            Self::TB_s(b) => (1000_u64.pow(4) as f64 * b).round() as usize,
            Self::TiB_s(b) => (1024_u64.pow(4) as f64 * b).round() as usize,
            Self::PB_s(b) => (1000_u64.pow(5) as f64 * b).round() as usize,
            Self::PiB_s(b) => (1024_u64.pow(5) as f64 * b).round() as usize,
            Self::ZB_s(b) => (1000_u64.pow(6) as f64 * b).round() as usize,
            Self::ZiB_s(b) => (1024_u64.pow(6) as f64 * b).round() as usize,
        }
    }
}

/// Trait use for casting from primitive type with a clear syntax
#[allow(non_snake_case)]
pub trait BWUnit {
    fn Byte_s(&self) -> BW;
    fn kB_s(&self) -> BW;
    fn KiB_s(&self) -> BW;
    fn MB_s(&self) -> BW;
    fn MiB_s(&self) -> BW;
    fn GB_s(&self) -> BW;
    fn GiB_s(&self) -> BW;
    fn TB_s(&self) -> BW;
    fn TiB_s(&self) -> BW;
    fn PB_s(&self) -> BW;
    fn PiB_s(&self) -> BW;
    fn ZB_s(&self) -> BW;
    fn ZiB_s(&self) -> BW;
}

#[allow(non_snake_case)]
impl<T: num::ToPrimitive> BWUnit for T {
    fn Byte_s(&self) -> BW {
        BW::Byte_s(self.to_usize().unwrap())
    }
    fn kB_s(&self) -> BW {
        BW::kB_s(self.to_f64().unwrap())
    }
    fn KiB_s(&self) -> BW {
        BW::KiB_s(self.to_f64().unwrap())
    }
    fn MB_s(&self) -> BW {
        BW::MB_s(self.to_f64().unwrap())
    }
    fn MiB_s(&self) -> BW {
        BW::MiB_s(self.to_f64().unwrap())
    }
    fn GB_s(&self) -> BW {
        BW::GB_s(self.to_f64().unwrap())
    }
    fn GiB_s(&self) -> BW {
        BW::GiB_s(self.to_f64().unwrap())
    }
    fn TB_s(&self) -> BW {
        BW::TB_s(self.to_f64().unwrap())
    }
    fn TiB_s(&self) -> BW {
        BW::TiB_s(self.to_f64().unwrap())
    }
    fn PB_s(&self) -> BW {
        BW::PB_s(self.to_f64().unwrap())
    }
    fn PiB_s(&self) -> BW {
        BW::PiB_s(self.to_f64().unwrap())
    }
    fn ZB_s(&self) -> BW {
        BW::ZB_s(self.to_f64().unwrap())
    }
    fn ZiB_s(&self) -> BW {
        BW::ZiB_s(self.to_f64().unwrap())
    }
}

/// Data is considered as dimentionless => [Data]*[Data] = [Data]
impl std::ops::Mul<f64> for BW {
    type Output = BW;

    fn mul(self, rhs: f64) -> Self {
        // TODO? conserve original unit ?
        let lhs_b_s = self.into_byte_s();
        let mul_b_s = (lhs_b_s as f64 * rhs).round() as usize;
        Self::Byte_s(mul_b_s)
    }
}
impl std::ops::Div<f64> for BW {
    type Output = BW;

    fn div(self, rhs: f64) -> Self {
        // TODO? conserve original unit ?
        let lhs_b_s = self.into_byte_s();
        let div_b_s = (lhs_b_s as f64 / rhs).round() as usize;
        Self::Byte_s(div_b_s)
    }
}
impl std::ops::Add for BW {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        let lhs_b = self.into_byte_s();
        let rhs_b = rhs.into_byte_s();
        Self::Byte_s(lhs_b + rhs_b)
    }
}

impl std::ops::Sub for BW {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        let lhs_b = self.into_byte_s();
        let rhs_b = rhs.into_byte_s();
        Self::Byte_s(lhs_b - rhs_b)
    }
}
