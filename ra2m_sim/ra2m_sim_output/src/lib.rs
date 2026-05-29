//! Handle ra2m outputs
//!
//! RA2M provide two main kind of outputs:
//!  * log: through the use of the log! macro. No requirement on the logged variables
//!  * trace: through the use of the trace! macro. Depending on the used backend, the tracced
//!    variables must implement particular Traits
//!
//!  # Log
//!  Log messages provide a highly tunable verbosity level on a global/per component basis.
//!  They are use to inform the advanced of the simulation and some strange behavior observed (eg
//!  Verb{Err,Warn,Info}.
//!  Lower Verbosity (eg Verb{Trace,Debug}) are mainly used during the creation of new models to
//!  help hunting issues.
//!  Those low level log entries are hidden behind feature flag "log-debug' and disabled by default.
//!
//!  # Trace
//!  Trace message also provide a tunable output on global/per component basis.
//!  Unlike the log messages, they are used during simulation analyses to extract performances
//!  metrics.
//!  Trace gather history information of event across the architectures and should be easy to
//!  post-process.
//!
//!  Trace/Log macros and filtering capability is public and depicts the user API.
//!  The backend used to dump content on disk is hide from the user.
//!  Thus, they could be change/enhance without breaking change in the component library.
//!
pub mod prelude;

pub mod log;
pub mod trace;

use chrono::Local;
use getset::Getters;
use once_cell::sync::OnceCell;
use std::os::unix::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Expand Home char in unix path
pub fn expand_unix_path(path: &str) -> PathBuf {
    let home_folder = std::env::var("HOME").unwrap();
    let expanded_path = path.replace('~', &home_folder);
    Path::new(&expanded_path).to_path_buf()
}

/// Created a timestamped log/trace output folder and simlink for ease of access
pub fn create_output_folder(path: &str) -> PathBuf {
    // expanded special unix char
    let base_path = expand_unix_path(path);
    if base_path.is_file() {
        panic!(
            "Try to create output with filename instead of dirname {}",
            base_path.display()
        );
    }

    // Created timestamped folder
    let timestamp = Local::now().format("%F_%H-%M-%S").to_string();
    let timestamped_path = base_path.join(Path::new(&timestamp));
    std::fs::create_dir_all(timestamped_path.clone()).unwrap();

    // Simlink for ease of access
    let symlink_path = base_path.join("latest_output");
    if symlink_path.is_symlink() {
        // Symlink already exits, delete it
        std::fs::remove_file(symlink_path.clone()).unwrap();
    }
    match fs::symlink(timestamped_path.clone(), symlink_path) {
        Ok(_) => {}
        Err(err) => {
            println!(
                "Failed to setup output symlink on {} => {}",
                timestamped_path.display(),
                err
            );
        }
    };
    timestamped_path
}

/// Global output state
/// Trace have distributed backend (e.g. per module backend dedicated to any Kind)
/// This structure only contain the global activation state and the trace folder
/// to ease the setup of backend on the run
/// Indeed, backend are created only when required (at first write in them) to prevent creation of
/// a bunch of useless file at startup.
#[derive(Debug, Getters)]
pub struct Output {
    /// Path of main output folder
    output_folder: PathBuf,
    /// Wrap in OnceCell for Lazy initialization
    trace_folder: OnceCell<PathBuf>,

    /// Global trace status
    trace_is_on: AtomicBool,
    /// Global log status
    log_is_on: AtomicBool,

    /// Filtering struct for global log messages
    #[getset(get = "pub")]
    log_filter: log::LogFilter,

    /// Log message Backend
    #[getset(get = "pub")]
    log_backend: Mutex<log::Backend>,
}

impl Output {
    pub fn init(path: &str) {
        let output_path = create_output_folder(path);

        // Ensure that OnceCell is empty
        OUTPUT
            .set(Output {
                output_folder: output_path.clone(),
                trace_folder: OnceCell::new(),
                trace_is_on: AtomicBool::new(false),
                log_is_on: AtomicBool::new(false),
                log_filter: Default::default(),
                log_backend: Mutex::new(log::Backend::new(&output_path)),
            })
            .expect("OUTPUT could be initialized only once");

        println!("Simulation output: {}", output_path.display());
    }

    pub fn global() -> &'static Output {
        OUTPUT
            .get()
            .expect("OUTPUT is not initialized. Did you run output::init(...)?")
    }

    pub fn get_trace_folder() -> &'static Path {
        let out = Self::global();
        out.trace_folder.get_or_init(|| {
            let trace_folder = out.output_folder.join(Path::new("traces"));
            std::fs::create_dir(trace_folder.clone()).unwrap();
            trace_folder
        })
    }

    /// Update trace_is_on value
    pub fn trace_update(is_on: bool) -> bool {
        let out = Self::global();
        out.trace_is_on.swap(is_on, Ordering::SeqCst)
    }

    /// Turn trace ON helper
    pub fn trace_enable() -> bool {
        Self::trace_update(true)
    }

    /// Turn trace OFF helper
    pub fn trace_disable() -> bool {
        Self::trace_update(false)
    }

    /// Get trace state
    pub fn trace_is_on() -> bool {
        if let Some(out) = OUTPUT.get() {
            out.trace_is_on.load(Ordering::SeqCst)
        } else {
            false
        }
    }

    /// Update log_is_on value and global log filtering
    pub fn log_update(is_on: bool, filter: Option<log::LogFilter>) -> bool {
        let out = Self::global();

        if let Some(lf) = filter {
            out.log_filter.replace(&lf);
        }
        out.log_is_on.swap(is_on, Ordering::SeqCst)
    }

    /// Turn log ON helper
    pub fn log_enable() -> bool {
        Self::log_update(true, None)
    }

    /// Turn log OFF helper
    pub fn log_disable() -> bool {
        Self::log_update(false, None)
    }

    /// Get log state
    pub fn log_is_on() -> bool {
        if let Some(out) = OUTPUT.get() {
            out.log_is_on.load(Ordering::SeqCst)
        } else {
            false
        }
    }
}
/// Output is backed by a global variable wrapped in once_cell
pub static OUTPUT: OnceCell<Output> = OnceCell::new();
