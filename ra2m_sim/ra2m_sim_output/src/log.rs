//! Define logging interface for ra2m
//!
//! Logging system is split in two half:
//!  * frontend (User API used in module library)
//!  * backend (How log message are dumped)
//!
//! # FrontEnd API
//! ## Verbosity and Category
//! Log message are registered with an associated Category and a level of Verbosity.
//!
//! The Verbosity level and enabled Category are managed by the user through regex that match with
//! Module path:
//! ` "regex_matching_on_path::[list of Category::Verbosity tuple]`
//! Through this kind of regex, only the Module which path match the regex will have there logging
//! attribute changed.
//! This enable a fine grain control of the Category::Verbosity over the architecture Module path.
//!
//! NB: Verbosity toggle have a smaller likelihood than message logging. Thus, the log_filter state
//! will be spread on a Module basis:
//!  * Fast check for logging (likely operation)
//!  * Long update procedure (unlikely operation)
//!
//!  ## Message structure
//!  Log message will have a form of list of variables name to log instead of a formatted String.
//!  This will enable efficient backend in different format (eg. binary, db, ...). However, it is
//!  also be possible to add textual information through logging of String.
//!
//! # Runtime toggle
//! Log verbosity could be modified dynamically (eg. during simulation run) to enable fine grain
//! control around the RegionOfInterest. This could be done through top level and regex based
//! expression or from Module internal (only for the Module scope).
//!
//! Furthermore, a global toggle is also possible. Don't change the Module logging configuration,
//! only globally inhibitted the log.
//!
//!
//! # Backend
//! The frontend log API must be agnostic of the associated backend. This should enable
//! modification of the logging output format (txt, binary, sgbd) without impacting the library of
//! Module.
//! At the beginning only a simple textual output will be provided.
//!

use getset::Getters;
use std::{
    fmt,
    fs::File,
    io::{Error, ErrorKind},
    path::Path,
    sync::RwLock,
};
use strum::{EnumCount, IntoEnumIterator};
use strum_macros::{EnumCount, EnumIter, EnumString};

/// Verbosity backed by usize to made easy comparison.
/// Error is the greatest one and couldn't be disabled
#[derive(
    Debug, Default, Copy, Clone, Eq, PartialEq, PartialOrd, EnumCount, EnumIter, EnumString,
)]
#[repr(usize)]
#[allow(non_camel_case_types)]
pub enum Verbosity {
    Debug,
    Trace,
    #[default]
    Info,
    Warning,
    Error,
}

/// List of available log category
/// Backed by usize to easily extract the number of variant and used in LogFilter as array index
#[derive(Debug, Default, Copy, Clone, EnumCount, EnumIter, EnumString)]
#[repr(usize)]
pub enum Category {
    /// Log module messages
    Own,
    /// Log port messages
    Port,
    /// Log protocol/payload messages
    Protocol,
    /// Log core::scheduler messages
    Scheduler,
    /// Log test messages
    Test,
    /// Log generic messages that couldn't fit in any other categories
    #[default]
    Any,
}

impl Category {
    fn as_index(&self) -> usize {
        *self as usize
    }
}

/// Args for log module
/// Args for log that should be parsed through CLI => impl FromStr trait
#[derive(Clone, Debug, Getters)]
#[getset(get = "pub")]
pub struct Args {
    path: String,
    dflt_verb: Verbosity,
    cat_verb: Vec<(Category, Verbosity)>,
}

impl std::str::FromStr for Args {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path_dfltcatverb = s.split('=').collect::<Vec<&str>>();
        if path_dfltcatverb.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid split, expected \"path=dflt_verbosity:{category:verbosity, ...}\"",
            ));
        }

        // Parse path
        let path = path_dfltcatverb[0].to_owned();

        // Parse default and catverb vector
        let dflt_catverb = path_dfltcatverb[1].split_once(':').unwrap();
        let dflt_verb = Verbosity::from_str(dflt_catverb.0.trim()).unwrap();

        // Parse catverb as vector of Category:Verbosity
        let mut cat_verb = Vec::new();
        if !dflt_catverb.1.is_empty() && dflt_catverb.1 != "{}" {
            for cv in dflt_catverb
                .1
                .strip_prefix('{')
                .unwrap()
                .strip_suffix('}')
                .unwrap()
                .split(',')
            {
                if cv.is_empty() {
                    // Case of trailing ','
                    continue;
                }
                let raw_cv = cv.split(':').collect::<Vec<&str>>();
                if raw_cv.len() != 2 {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "Invalid split, expected \"Category:Verbosity\"",
                    ));
                }
                let cat = Category::from_str(raw_cv[0].trim()).unwrap();
                let verb = Verbosity::from_str(raw_cv[1].trim()).unwrap();
                cat_verb.push((cat, verb));
            }
        }
        Ok(Self {
            path,
            dflt_verb,
            cat_verb,
        })
    }
}

/// Contain Category::Verbosity correspondence
/// Have set/get method for per category access and global replace.
#[derive(Default)]
pub struct LogFilter {
    bank: RwLock<[Verbosity; Category::COUNT]>,
}

impl LogFilter {
    pub fn new(dflt_verbosity: Verbosity) -> Self {
        LogFilter {
            bank: RwLock::new([dflt_verbosity; Category::COUNT]),
        }
    }

    pub fn get(&self, category: Category) -> Verbosity {
        let bank = self.bank.read().unwrap();
        bank[category.as_index()]
    }

    pub fn set(&self, category: Category, verbosity: Verbosity) {
        let mut bank = self.bank.write().unwrap();
        bank[category as usize] = verbosity;
    }

    pub fn replace(&self, filter: &LogFilter) {
        let mut bank = self.bank.write().unwrap();
        let trgt = filter.bank.read().unwrap();
        for cat in Category::iter() {
            bank[cat.as_index()] = trgt[cat.as_index()];
        }
    }

    pub fn should_log(&self, category: Category, verbosity: Verbosity) -> bool {
        let bank = self.bank.read().unwrap();
        bank[category as usize] <= verbosity
    }
}

impl Clone for LogFilter {
    fn clone(&self) -> Self {
        let clone = Self::new(Default::default());

        let mut clone_wr = clone.bank.write().unwrap();
        let bank = self.bank.read().unwrap();
        for cat in Category::iter() {
            clone_wr[cat.as_index()] = bank[cat.as_index()];
        }
        drop(clone_wr);
        clone
    }
}

impl fmt::Debug for LogFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("LogFilter");
        for cat in Category::iter() {
            dbg.field(&format!("{cat:?}"), &self.get(cat));
        }
        dbg.finish()
    }
}

impl fmt::Display for LogFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LogFilter: [")?;
        for cat in Category::iter() {
            write!(f, "{:?}::{:?}", cat, &self.get(cat))?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

/// Define LogBackend type that define the underlying log backend based on
/// enabled feature. Available options are:
///   * TextLog [Default]
///     Simple backend that dump log in a file as text
///   * BinaryLog [Not available yet]
///     Backend that dump log content in binary form for reducing size on disk
///     => Required post-processing to read
pub type Backend = TextBackend;

#[derive(Debug)]
pub struct TextBackend {
    file: File,
}

impl TextBackend {
    pub fn new(output_dir: &Path) -> Self {
        let log_path = output_dir.join(Path::new("ra2m.log"));
        let file = File::create(log_path).unwrap();
        Self { file }
    }

    pub fn wb(&mut self) -> std::io::BufWriter<File> {
        std::io::BufWriter::new(self.file.try_clone().unwrap())
    }
}

/// Define Lowest supported verbosity.
///  Disable ultra-verbose entries for non log-debug build
#[cfg(feature = "log-debug")]
pub const LOWEST_VERBOSITY: Verbosity = Verbosity::Debug;
#[cfg(not(feature = "log-debug"))]
pub const LOWEST_VERBOSITY: Verbosity = Verbosity::Info;

/// Provide logging macro that ease log message generation
///
/// This macro have various matching option (NB: ()? -> optional fields):
///  1. log!( |self| category?, verbosity?
///     => comma separated list of variables to log
///     (=> user custom str or format!(...) )?
///     The passed self must implement the Module trait.  These kind of log message use the module
///     scope to filtered the message content (NB: self must implement Module trait)
///
///  2. log!( category?, verbosity?
///     => comma separated list of variables to log
///     (=> user custom str or format!(...) )?
///     These kind of log message use the global scope to filtered the message content
#[macro_export]
macro_rules! log {

    // Use local module scope for filtering
    ($props: ident, $($category: expr)?, $($verbosity: expr)? => $($var: expr$(,)?)* $(=> $msg: tt)? ) => {{
        if Output::log_is_on() {
            // Handle default category and verbosity
            // NB: Small hack to prevent warning on mutability. This could be replaced if we found an
            // easy way to check the presence of optional fragments
            let cat: log::Category;
            let verb: log::Verbosity;
            default_catverb!(cat, verb, $($category)?, $($verbosity)?);

            if verb >= log::LOWEST_VERBOSITY {
                // Get module log filter, if needed lock log_backend and generate log message
                let log_filter = $props.log_filter();
                if log_filter.should_log(cat, verb) { // Log enable @Module level
                    let path = $props.path();
                    let out = Output::global();
                    let mut backend = out.log_backend().lock().unwrap();
                    let mut wb = backend.wb();
                    log_format!(wb, cat, verb, path => $($var,)* $(=> $msg)?);
                }
            }
        }
    }};

    // Syntax sugar to path directly a self that implement `Module` instead of properties
    // Use local module scope for filtering
    // self could be anything that implement fn properties -> &Arc<module::Properties>
    (|$self: ident| $($category: expr)?, $($verbosity: expr)? => $($var: expr$(,)?)* $(=> $msg: tt)? ) => {{
        let props = $self.properties();
        log!(props, $($category)?, $($verbosity)? => $($var)* $(=> $msg)?)
    }};

    // Use global scope for filtering
    ($($category: expr)?, $($verbosity: expr)? => $($var: expr$(,)?)* $(=> $msg: tt)? ) => {{
        if Output::log_is_on() {
            // Handle default category and verbosity
            // NB: Small hack to prevent warning on mutability. This could be replaced if we found an
            // easy way to check the presence of optional fragments
            let cat: log::Category;
            let verb: log::Verbosity;
            default_catverb!(cat, verb, $($category)?, $($verbosity)?);

            if verb >= log::LOWEST_VERBOSITY {
                let out = Output::global();
                if out.log_filter().should_log(cat, verb) {
                    let mut backend = out.log_backend().lock().unwrap();
                    let mut wb = backend.wb();
                    log_format!(wb, cat, verb => $($var,)* $(=> $msg)?);
                }
            }
        }
    }};
}

#[macro_export]
macro_rules! default_catverb {
    ($catId: ident, $verbId: ident, $cat: expr, $verb: expr) => {
        $catId = $cat;
        $verbId = $verb;
    };
    ($catId: ident, $verbId: ident, $cat: expr, ) => {
        $catId = $cat;
        $verbId = Default::default();
    };
    ($catId: ident, $verbId: ident, , $verb:expr) => {
        $catId = Default::default();
        $verbId = $verb;
    };
    ($catId: ident, $verbId: ident, ,) => {
        $catId = Default::default();
        $verbId = Default::default();
    };
}

/// Provide log formatting macro.
/// NB: The passed wb is the BufWriter attached to the underlying backend.
/// NB': Currently log formatting is friendly toward textual log, but the provided abstraction
/// hide backend change from the user perspective (eg. same macro use and shape).
/// => Thus, in the future, this formatting macro could be extended to support others kind of
/// backend (binary, polars, ...)
#[macro_export]
macro_rules! log_format {
    // Log header and user message if any, then recurse on variables list
    ($wb:ident,  $cat: expr, $verb: expr $(,$path:tt)? => $($var: expr$(,)?)* $(=> $msg: tt)? ) => {
        // Macro hack to get function name
        // Extracted from crate stdext = "0.3.1"
        // Okay, this is ugly, I get it. However, this is the best we can get on a stable rust.
        fn log_format_f_hack() {}
        let caller_name = {
            fn type_name_of<T>(_: T) -> &'static str {
                std::any::type_name::<T>()
            }
            let name = type_name_of(log_format_f_hack);
            // `32` is the length of the `::{{closure}}::log_format_f_hack`.
            &name[..name.len() - 32]
        };
        writeln!($wb, "<").unwrap();
        let tick = if let Some(tick) = time::TimeKeeper::try_cur_tick() { format!("{}", tick) } else { "Unstarted".into() };
        write!($wb, "\t[{}][{}:{}, {}, {:?}::{:?}]", tick, std::file!(), std::line!(), caller_name, $cat, $verb).unwrap();
        $(write!($wb, "\n\t=> path: {},", $path).unwrap();)?
        $crate::log_format!($wb, $($var,)*);
        $(write!($wb, "\n\t=> {}.", $msg).unwrap();)?
        writeln!($wb, "").unwrap();
        writeln!($wb, ">").unwrap();
    };

    // Log variables
    ($wb: ident, $($var: expr$(,)? )*) => {
        write!($wb, "\n\t=> {{").unwrap();
        $(write!($wb, " {} = {:?},", std::stringify!($var), $var).unwrap();)*
        write!($wb, "}}").unwrap();
    };
}

#[cfg(test)]
mod units_tests {
    use super::*;

    #[test]
    fn test_log_filter() {
        let lf: LogFilter = Default::default();

        // Test should_log function
        for verb in Verbosity::iter() {
            for cat in Category::iter() {
                lf.set(cat, verb);

                for use_verb in Verbosity::iter() {
                    assert_eq!(use_verb >= lf.get(cat), lf.should_log(cat, use_verb));
                }
            }
        }

        // test replace function
        lf.replace(&LogFilter::new(Verbosity::Warning));
        for cat in Category::iter() {
            assert_eq!(lf.get(cat), Verbosity::Warning);
        }
    }
}
