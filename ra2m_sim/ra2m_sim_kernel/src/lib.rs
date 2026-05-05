#![feature(linked_list_cursors)]

//! Contain simulation kernel of Ra2m:
//!
//! * delay: Custom future used by Hw process to wait for a given amount of simulated time
//! * event: Custom future used by Hw process to wait/regiter or trigger an Hw event
//! * module: Define module abstraction that ease the generic management of distinct module
//! definition (eg. Module of different kind with distinct behavior)
//! * port: Inter-process communication primitive. Rely on async channel and correctly hooked up
//! * within the scheduler to prevent deadlock
//! * memport: Leverage port abstraction to build memory mapped inter-process communication (eg. request/response protocol)
//! * scheduler: provide utility function for Hw process interaction and a simulate function that
//!   run the simulation for a given amount of time
//!
pub mod prelude;

pub mod delay;
pub mod event;
pub mod module;
pub mod port;
pub mod protocol;
pub mod scheduler;
pub mod time;
pub mod types;
pub mod unit;

/// Default scheduler capacity used by Unit/Integration -tests
pub const DFLT_HW_PROCESS: usize = 400;

/// Init the simulation global state and return Scheduler
pub fn init_simulation(
    start_tick: time::Tick,
    timescale: unit::Time,
    timing_mode: time::TimingMode,
    max_hw_process: usize,
) -> scheduler::Scheduler {
    #[cfg(feature = "ctrlc-panic")]
    {
        match ctrlc::set_handler(|| {
            use std::sync::atomic::Ordering;

            let shared_api = scheduler::SchedulerAPI::global();
            println!(" @Tick {} => {:?}", cur_tick(), shared_api,);

            shared_api.force_early_exit.store(true, Ordering::SeqCst);
        }) {
            Ok(_) => {}
            Err(err) => {
                println!("Error setting Ctrl-C handler {err}");
            }
        };
    }

    // Register a custom panic hook to stop simulation on first panic in thread
    // take_hook() returns the default hook in case when a custom one is not set
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // invoke the default handler and exit the process
        orig_hook(panic_info);
        std::process::exit(1);
    }));

    // Register/Init global state
    time::TimeKeeper::reset(start_tick, timescale, timing_mode);
    let max_hw_process = scheduler::SchedulerAPI::reset(max_hw_process);
    scheduler::Scheduler::new(max_hw_process)
}

/// Generic way to handle log_args from user
pub fn user_log_args(
    root: &std::sync::Arc<module::Area>,
    log_args: Option<Vec<ra2m_sim_output::log::Args>>,
) {
    // Apply user log-args recursively
    if let Some(args) = log_args {
        for arg in args {
            let reached = root.apply_log_arg(&arg);
            println!("Apply {arg:?} have reached {reached} modules");
        }

        // TODO add flag to delay log activation
        ra2m_sim_output::Output::log_enable();
    }
    // TODO how to setup global log level ?
}

/// Generic way to handle trace_args from user
pub fn user_trace_args(
    root: &std::sync::Arc<module::Area>,
    trace_args: Option<Vec<ra2m_sim_output::trace::Args>>,
) {
    // Apply user trace-args recursively
    if let Some(args) = trace_args {
        for arg in args {
            let reached = root.apply_trace_arg(&arg);
            println!("Apply {arg:?} have reached {reached} modules");
        }

        // TODO add flag to delay log activation
        ra2m_sim_output::Output::trace_enable();
    }
}

// Function redefinition to reduce boilerplate module naming. Expose submodule function at the
// module level
/// Export current simulation tick getter at module level
pub fn cur_tick() -> time::Tick {
    time::TimeKeeper::cur_tick()
}
