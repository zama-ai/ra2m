//! Define trace interface for ra2m
//!
//! Trace system is split in two half:
//!  * frontend (User API used in module library)
//!  * backend (How log message are dumped)
//!
//! # FrontEnd API
//! ## Kind
//! Trace message are dumped with an associated Kind:
//!   * Payload => Trace build through lifetime of a payload. Every Req/Resp
//!     Handler add their own set of information
//!   * Pipeline => Trace build through component pipeline. Every pipeline stage
//!     add their own set of information
//!
//! The enabled Kind are managed by the user through regex that match with Module path:
//! ` "regex_matching_on_path::[list of Category::Verbosity tuple]`
//! Through this kind of regex, only the Module which path match the regex will have there logging
//! attribute changed.
//! This enable a fine grain control of generated trace over the architecture Module path.
//!
//! Trace message already aggregate information from various handler. Thus, it could be store on a per stream basis.
//! => Every module with active trace have it's own trace backend with a dedicated file per Kind.
//!
//! ## Runtime toggle
//! Trace kind could be modified dynamically (eg. during simulation run) to enable fine grain
//! control around the RegionOfInterest. This could be done through regex based expression or from
//! Module internal.
//!
//! # Backend
//! The frontend log API must be agnostic of the associated backend. This should enable
//! modification of the logging output format (txt, polars) without impacting the library of
//! Module.
//! At the beginning only a simple textual (i.e. json) output will be provided.
//!

use super::*;
use getset::{Getters, MutGetters};
use std::mem::{self, MaybeUninit};
use std::{
    fmt,
    fs::File,
    io::{BufWriter, Error, ErrorKind},
    ops::Index,
    path::Path,
    sync::Mutex,
};
use strum::{EnumCount, IntoEnumIterator};
use strum_macros::{EnumCount, EnumIter, EnumString};

/// List of available trace kind
/// Backed by usize to easily extract the number of variant and used in TraceFilter as array index
#[derive(Debug, Copy, Clone, EnumCount, EnumIter, EnumString)]
#[repr(usize)]
pub enum Kind {
    /// Trace generated during Port::Packet::Payload lifetime
    Payload,
    /// Trace generated during component pipeline
    Pipeline,
}

impl Kind {
    fn as_index(&self) -> usize {
        *self as usize
    }
}

/// Args for trace module
/// Args for trace that should be parsed through CLI => impl FromStr trait
#[derive(Clone, Debug, Getters)]
#[getset(get = "pub")]
pub struct Args {
    path: String,
    kind_val: Vec<(Kind, bool)>,
}

impl std::str::FromStr for Args {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path_kind = s.split('=').collect::<Vec<&str>>();
        if path_kind.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid split, expected \"path={kind:val, ...}\"",
            ));
        }

        // Parse path
        let path = path_kind[0].to_owned();

        // Parse kind as vector of kind:val
        let mut kind_val = Vec::new();
        for kv in path_kind[1]
            .strip_prefix('{')
            .unwrap()
            .strip_suffix('}')
            .unwrap()
            .split(',')
        {
            if kv.is_empty() {
                // Case of trailing ','
                continue;
            }
            let raw_kv = kv.split(':').collect::<Vec<&str>>();
            if raw_kv.len() != 2 {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "Invalid split, expected \"Kind:bool\"",
                ));
            }
            let kind = Kind::from_str(raw_kv[0].trim()).unwrap();
            let val = raw_kv[1].parse::<bool>().unwrap();

            kind_val.push((kind, val));
        }
        Ok(Self { path, kind_val })
    }
}

#[derive(Debug, Getters)]
pub struct Backend {
    #[getset(get = "pub")]
    is_on: bool,
    filename: String,
    /// Wrap in Option for Lazy initialization
    writer: Option<BufWriter<File>>,
}

impl Backend {
    pub fn new(is_on: bool, pathname: &str, kind: Kind) -> Self {
        let filename = format!("{pathname}_{kind:?}.trace.bincode");
        Self {
            is_on,
            filename,
            writer: None,
        }
    }

    pub fn wtr(&mut self) -> &mut BufWriter<File> {
        if self.writer.is_none() {
            let trace_folder = Output::get_trace_folder();
            let trace_path = trace_folder.join(Path::new(&self.filename));
            self.writer = Some(BufWriter::new(File::create(trace_path).unwrap()));
        }
        self.writer.as_mut().unwrap()
    }
}

/// Contain Kind status and associated backend
/// Backend have lazy setup and open-file only on first trace stream access
#[derive(Getters, MutGetters)]
pub struct HwTrace {
    /// Logging backend
    #[getset(get = "pub")]
    backend: [Mutex<Backend>; Kind::COUNT],
}

impl HwTrace {
    pub fn new(is_on: bool, pathname: &str) -> Self {
        let backend = {
            // Create an uninitialized array of `MaybeUninit`. The `assume_init` is
            // safe because the type we are claiming to have initialized here is a
            // bunch of `MaybeUninit`s, which do not require initialization.
            let mut data: [MaybeUninit<Mutex<Backend>>; Kind::COUNT] =
                unsafe { MaybeUninit::uninit().assume_init() };

            // Dropping a `MaybeUninit` does nothing. Thus using raw pointer
            // assignment instead of `ptr::write` does not cause the old
            // uninitialized value to be dropped. Also if there is a panic during
            // this loop, we have a memory leak, but there is no memory safety
            // issue.
            for (k, elem) in std::iter::zip(Kind::iter(), data[..].iter_mut()) {
                elem.write(Mutex::new(Backend::new(is_on, pathname, k)));
            }

            // Everything is initialized. Transmute the array to the
            // initialized type.
            unsafe {
                mem::transmute::<
                    [MaybeUninit<Mutex<Backend>>; Kind::COUNT],
                    [Mutex<Backend>; Kind::COUNT],
                >(data)
            }
        };
        HwTrace { backend }
    }

    /// Update is_on value
    pub fn update(&self, kind: Kind, is_on: bool) -> bool {
        let mut backend = self[kind].lock().unwrap();
        let prv_is_on = backend.is_on;
        backend.is_on = is_on;
        prv_is_on
    }

    /// Turn trace ON helper
    pub fn enable(&self, kind: Kind) -> bool {
        self.update(kind, true)
    }

    /// Turn trace OFF helper
    pub fn disable(&self, kind: Kind) -> bool {
        self.update(kind, false)
    }
}

impl Index<Kind> for HwTrace {
    type Output = Mutex<Backend>;

    fn index(&self, kind: Kind) -> &Self::Output {
        &self.backend[kind.as_index()]
    }
}

impl fmt::Debug for HwTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("HwTrace");
        for kind in Kind::iter() {
            dbg.field(&format!("{kind:?}"), &self[kind]);
        }
        dbg.finish()
    }
}

/// Provide trace macro that ease hide backend abstraction from user.
///
/// This macro only support one matching option:
///  trace!(|self|, Kind => var)
///  where:
///   self must implement the Module trait.
///   var must implement Trace trait
///
#[macro_export]
macro_rules! trace {
    (|$self: ident| $kind: expr => $([$header: expr])? $var: ident ) => {{
        // Append last history stages
        // History could be used outside of trace and should be kept consistent
        $var.wrap_up(*$self.properties().uid());

        // Check global trace status
        if Output::trace_is_on() {
            let backend = &$self.properties().hw_trace()[$kind];
            let mut backend = backend.lock().unwrap();
            if *backend.is_on() {
                // Wrap header in tuple if any
                let header_msg = ($(&$header,)* &$var);
                let mut wtr = backend.wtr();
                bincode::serialize_into(&mut *wtr, &header_msg).unwrap();
                writeln!(wtr, "").unwrap();
            }
        }
    }};
}
