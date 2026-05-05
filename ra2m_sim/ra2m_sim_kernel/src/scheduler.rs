//! Manage Hw process and simulated time
//! Hook simulation time and Hw process in rust async runtimu (eg. Tokio currently used)
//!
//! Provide utilities function to enable Hw process to communicate with the scheduler through
//! Order.
//! Scheduler read order, schedule new tasks and manage simulation delta-cycles.
//! When no process is running, scheduler pop next task, refresh delta-cycle properties and update
//! simulation time
//!

use super::*;

use ra2m_sim_output::log::{Category, Verbosity};
use ra2m_sim_output::{default_catverb, log, log_format, Output};

use crossbeam::atomic::AtomicCell;
use crossbeam::queue::ArrayQueue;
use once_cell::sync::OnceCell;
use std::sync::atomic::AtomicBool;
use std::{
    collections::{BTreeMap, BinaryHeap},
    fmt,
    future::Future,
    io::Write, // use by log macro
    pin::Pin,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

#[derive(Debug)]
pub enum ExitKind {
    NoHwPrc,
    EndOfTick,
    PrcExit,
}

/// Order sent by Hw process to the scheduler
#[derive(Debug)]
pub enum Order {
    WaitUntil(time::Tick, Waker),
    WaitOn(usize, Waker),
    NotifyAt(time::Tick, usize),
    Exit(ExitKind),
}

/// Future task to be process by the scheduler at a given simulation time
/// Implement Ord to enable efficient access of next available task in an ordered manner from
/// BinaryHeap structure.
#[derive(Debug)]
pub enum Task {
    Wakeup(time::Tick, Waker),
    Notify(time::Tick, usize),
}

impl Task {
    fn tick(&self) -> time::Tick {
        match self {
            Task::Wakeup(t, _) => *t,
            Task::Notify(t, _) => *t,
        }
    }
}

impl Ord for Task {
    /// Order Task in reverse order based only on their tick field
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tick().cmp(&other.tick()).reverse()
    }
}

impl PartialOrd for Task {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.tick() == other.tick()
    }
}
impl Eq for Task {}

/// WaitHwThread
/// Future with particular handle to wake up scheduler when required.
/// Wait until all alive_prc were computed and register for the given delta_cycle
pub struct WaitHwThread;

impl Future for WaitHwThread {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if SchedulerAPI::delta_cycle_ended() {
            Poll::Ready(())
        } else {
            // Register waker in the Scheduler for future access by HwTask
            SchedulerAPI::register_waker(cx.waker().clone());
            Poll::Pending
        }
    }
}

// Handle Job lifecycle and communication struct between Module/Scheduler
/// Scheduler shared API: It provides utilities function that enable Hw process
/// to interact with the scheduler in an efficient manner (lock-free)
pub struct SchedulerAPI {
    /// Available Hw process
    pub hw_prc: AtomicUsize,
    /// Alived Hw process
    pub alive_prc: AtomicIsize,
    /// Locked Hw process during Delta-cycle
    pub locked_prc: AtomicUsize,
    /// Unlocked Hw process during Delta-cycle
    pub unlocked_prc: AtomicUsize,
    /// Unlock event during Delta-cycle
    pub unlock_evt: AtomicUsize,

    /// Order queue from Hw process
    pub order_q: ArrayQueue<scheduler::Order>,
    /// Waker reference to enable Hw process to wakeup the scheduler when needed
    pub waker: AtomicCell<Option<Waker>>,

    /// Handle early shutdown request from user
    /// Try to cleanly shutdown if possible
    pub force_early_exit: AtomicBool,
}

impl SchedulerAPI {
    pub fn reset(max_hw_process: usize) -> usize {
        // Populate the once_cell instance if needed
        // NB: Testing required spawning of multiple simulation sequentially
        // This construct enable multiple setup of the once_cell content without issues
        let shared_api = SCHEDULER_API.get_or_init(|| SchedulerAPI {
            hw_prc: AtomicUsize::new(0),
            alive_prc: AtomicIsize::new(0),
            locked_prc: AtomicUsize::new(0),
            unlocked_prc: AtomicUsize::new(0),
            unlock_evt: AtomicUsize::new(0),
            order_q: ArrayQueue::new(max_hw_process),
            waker: AtomicCell::new(None),
            force_early_exit: AtomicBool::new(false),
        });
        // Reset SchedulerAPI values
        shared_api.alive_prc.store(0, Ordering::SeqCst);
        while !shared_api.order_q.is_empty() {
            shared_api.order_q.pop();
        }
        let capacity = shared_api.order_q.capacity();
        assert!(capacity >= max_hw_process);
        shared_api.waker.take();

        log!(Category::Scheduler, Verbosity::Info
                => capacity
                =>"Reset SchedulerAPI");
        capacity
    }

    /// Utility function that retrieved the global SchedulerAPI state
    pub fn global() -> &'static Self {
        SCHEDULER_API
            .get()
            .expect("Core Scheduler is not initialized")
    }

    /// Utility function that check if the current delta-cycle is over
    /// => all the alive_prc are registered back in the scheduler and the scheduler must be
    /// woken up
    pub fn delta_cycle_ended() -> bool {
        0 == SchedulerAPI::global().alive_prc.load(Ordering::SeqCst)
    }

    /// Utility function used by Hw process to send order to the scheduler.
    pub fn push_order(order: Order) {
        let api = SchedulerAPI::global();
        let halt_prc = match &order {
            Order::WaitUntil(t, _) => {
                debug_assert!(
                    *t >= time::TimeKeeper::cur_tick(),
                    "Push Order WaitUntil {} @{} [{} tick in the past]",
                    *t,
                    time::TimeKeeper::cur_tick(),
                    time::TimeKeeper::cur_tick() - *t
                );
                true
            }
            Order::NotifyAt(t, _) => {
                debug_assert!(
                    *t >= time::TimeKeeper::cur_tick(),
                    "Push Order NotifyAt {} @{} [{} tick in the past",
                    *t,
                    time::TimeKeeper::cur_tick(),
                    time::TimeKeeper::cur_tick() - *t
                );
                false
            }
            Order::Exit(_) => false,
            _ => true,
        };
        log!(Category::Scheduler, Verbosity::Debug
                => order, halt_prc);

        // Store order in the scheduler order queue
        api.order_q.push(order)
                    .expect("More process order than the defined maximum are generated. Check your simulation configuration or extend the `max_prc` value.");

        // Halt process only if process order required it
        // Eg. not for notification order
        if halt_prc {
            let pre_ops_alive = api.alive_prc.fetch_sub(1_isize, Ordering::SeqCst);
            if 1 == pre_ops_alive {
                // Wake the scheduler to triggered next delta-cycle
                if let Some(waker) = api.waker.take() {
                    waker.wake_by_ref();
                }
            }
        }
    }

    /// Register new Hw process in the scheduler
    /// Hw process are considered alived at startup (eg. until first await)
    pub fn register_new_prc() {
        let api = SchedulerAPI::global();
        api.hw_prc.fetch_add(1_usize, Ordering::SeqCst);
        api.alive_prc.fetch_add(1_isize, Ordering::SeqCst);
    }

    /// UnRegister Hw process from the scheduler
    /// Hw process are halted and removed from the list
    pub fn unregister_prc() {
        let api = SchedulerAPI::global();
        api.hw_prc.fetch_sub(1_usize, Ordering::SeqCst);
        let pre_ops_alive = api.alive_prc.fetch_sub(1_isize, Ordering::SeqCst);
        if 1 == pre_ops_alive {
            // Wake the scheduler to triggered next delta-cycle
            if let Some(waker) = api.waker.take() {
                waker.wake_by_ref();
            }
        }
    }

    /// Utility function to deferred task wake up to tokio runtime
    /// After wakeup from tokio, task is registered back in the HwScheduler
    /// Used for concurrency access over lock
    pub fn lock_prc() {
        let api = SchedulerAPI::global();
        api.locked_prc.fetch_add(1_usize, Ordering::SeqCst);
        let pre_ops_alive = api.alive_prc.fetch_sub(1_isize, Ordering::SeqCst);
        if 1 == pre_ops_alive {
            // Wake the scheduler to triggered next delta-cycle
            if let Some(waker) = api.waker.take() {
                waker.wake_by_ref();
            }
        }
    }

    pub fn unlock_prc() {
        let api = SchedulerAPI::global();
        let pre_ops_alive = api.alive_prc.fetch_add(1_isize, Ordering::SeqCst);
        api.locked_prc.fetch_sub(1_usize, Ordering::SeqCst);
        api.unlocked_prc.fetch_add(1_usize, Ordering::SeqCst);

        if -1 == pre_ops_alive {
            // Wake the scheduler to triggered next delta-cycle
            if let Some(waker) = api.waker.take() {
                waker.wake_by_ref();
            }
        }
    }

    pub fn unlock_evt() {
        let api = SchedulerAPI::global();
        api.unlock_evt.fetch_add(1_usize, Ordering::SeqCst);
    }

    /// Utility function to store the scheduler associated waker for later use
    pub fn register_waker(waker: Waker) {
        let waker_cell = &SchedulerAPI::global().waker;
        waker_cell.store(Some(waker));
    }
}

impl fmt::Debug for SchedulerAPI {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scheduler")
            .field("hw_prc", &self.hw_prc)
            .field("alive_prc", &self.alive_prc)
            .field("locked_prc", &self.locked_prc)
            .field("unlocked_prc", &self.unlocked_prc)
            .field("unlock_evt", &self.unlock_evt)
            .finish()
    }
}

/// SchedulerAPI is backed by a global variable wrapped in once_cell
pub static SCHEDULER_API: OnceCell<SchedulerAPI> = OnceCell::new();

/// Store the scheduler state and manage the simulation.
#[derive(Debug)]
pub struct Scheduler {
    wait_on_tick: BinaryHeap<Task>,
    wait_on_event: BTreeMap<usize, Vec<Waker>>,
}

impl Scheduler {
    /// Register a new `Scheduler` in the global once_cell
    ///
    /// `prc_max` Maximum number of Hardware process
    ///
    pub fn new(max_hw_process: usize) -> Self {
        Scheduler {
            wait_on_tick: BinaryHeap::with_capacity(max_hw_process),
            wait_on_event: BTreeMap::new(),
        }
    }

    /// Start scheduler for a given amount of simulated time (eg. Tick)
    pub async fn simulate(&mut self, duration: usize) -> (time::Tick, ExitKind) {
        //NB: Keep println! and log! for simulation start and simulation end
        println!(
            "{}: Start simulation loop for {} tick",
            time::TimeKeeper::cur_tick(),
            duration
        );
        log!(Category::Scheduler, Verbosity::Info
                => duration
                => "Start simulation loop");

        let shared_api = SchedulerAPI::global();

        loop {
            loop {
                // Wait for end of current delta-cycle
                match tokio::time::timeout(std::time::Duration::from_millis(500), WaitHwThread {})
                    .await
                {
                    Ok(_) => {
                        break;
                    }
                    Err(_) => {
                        log!(Category::Scheduler, Verbosity::Warning
                            => shared_api
                            => "Scheduler WaitHwThread timeout in delta_cycle resolution");
                        println!("Scheduler WaitHwThread timeout in delta_cycle resolution");

                        if shared_api.force_early_exit.load(Ordering::SeqCst) {
                            panic!("Force_early_exit required when Scheduler seems in a deadlock");
                        }
                    }
                };
            }

            // Enforce that all unlocked process during delta-cycle correctly resolved
            let unlock_evt = shared_api.unlock_evt.load(Ordering::SeqCst);
            if unlock_evt > 0 {
                // Wait for unlocked prc to to be rescheduled
                while unlock_evt > shared_api.unlocked_prc.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }

                // Wait unlocked prc to resolved (eg. reach another wait point)
                loop {
                    // Wait for end of current delta-cycle
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(500),
                        WaitHwThread {},
                    )
                    .await
                    {
                        Ok(_) => {
                            break;
                        }
                        Err(_) => {
                            log!(Category::Scheduler, Verbosity::Warning
                                => shared_api
                                => "Scheduler WaitHwThread timeout in unlock_evt resolution");
                            println!("Scheduler WaitHwThread timeout in unlock_evt resolution");
                        }
                    };
                }
            }
            // clear up counter
            shared_api.unlock_evt.swap(0, Ordering::SeqCst);
            shared_api.unlocked_prc.swap(0, Ordering::SeqCst);

            // Retrieved task from previous inflight prc
            while let Some(order) = shared_api.order_q.pop() {
                match order {
                    Order::WaitUntil(t, w) => {
                        debug_assert!(
                            t >= time::TimeKeeper::cur_tick(),
                            "Recv WaitUntil {} @{} [{} tick in the past",
                            t,
                            time::TimeKeeper::cur_tick(),
                            time::TimeKeeper::cur_tick() - t
                        );
                        let t = std::cmp::max(time::TimeKeeper::cur_tick() + 1, t); // TODO
                        log!(Category::Scheduler, Verbosity::Debug => t, w => "WaitUntil");
                        self.wait_on_tick.push(Task::Wakeup(t, w));
                    }
                    Order::NotifyAt(t, evt) => {
                        debug_assert!(
                            t >= time::TimeKeeper::cur_tick(),
                            "Recv NotifyAt {} @{} [{} tick in the past",
                            t,
                            time::TimeKeeper::cur_tick(),
                            time::TimeKeeper::cur_tick() - t
                        );
                        let t = std::cmp::max(time::TimeKeeper::cur_tick() + 1, t); // TODO
                        self.wait_on_tick.push(Task::Notify(t, evt));
                    }
                    Order::WaitOn(evt, w) => {
                        log!(Category::Scheduler, Verbosity::Debug => evt, w => "WaitOn");
                        // Key already present append waker to the already register wakers
                        if let Some(waker_vec) = self.wait_on_event.get_mut(&evt) {
                            waker_vec.push(w);
                        } else {
                            // insert a new entry
                            let waker_vec = vec![w];
                            self.wait_on_event.insert(evt, waker_vec);
                        }
                    }
                    Order::Exit(kind) => {
                        log!(Category::Scheduler, Verbosity::Info
                            => shared_api, kind
                            => "Scheduler Received exit order");
                        return (time::TimeKeeper::cur_tick(), kind);
                    }
                }
            }

            let next_tick;
            if let Some(task) = self.wait_on_tick.peek() {
                next_tick = task.tick();
            } else {
                // No more hw event to simulate
                println!(
                    "{}: Scheduler have no more prc to simulate",
                    time::TimeKeeper::cur_tick()
                );
                log!(Category::Scheduler, Verbosity::Info
                        => "Scheduler have no more prc to simulate");
                return (time::TimeKeeper::cur_tick(), ExitKind::NoHwPrc);
            }

            if next_tick > duration {
                // End of simulation time reached
                println!("{}: Reach end of simulation.", time::TimeKeeper::cur_tick());
                log!(Category::Scheduler, Verbosity::Info
                        =>
                        => "Reach end of simulation");
                return (time::TimeKeeper::cur_tick(), ExitKind::EndOfTick);
            }

            if shared_api.force_early_exit.load(Ordering::SeqCst) {
                //  Early exit request
                println!("{}: Simulation early exit.", time::TimeKeeper::cur_tick());
                log!(Category::Scheduler, Verbosity::Info
                        =>
                        => "Simulation early exit.");
                return (time::TimeKeeper::cur_tick(), ExitKind::EndOfTick);
            }

            // Update Time for next simulation cycle and reset triggered events
            time::TimeKeeper::update_tick(next_tick);
            event::EventKeeper::clear_triggered_events();

            // Loop over task that must be processed during the simulation cycle
            let mut woken_prc = 0_usize;
            log!(Category::Scheduler, Verbosity::Debug => shared_api);
            while let Some(task) = self.wait_on_tick.peek() {
                if next_tick < task.tick() {
                    break;
                }

                // Remove next task from the heap
                if let Some(task) = self.wait_on_tick.pop() {
                    match task {
                        Task::Wakeup(_, w) => {
                            woken_prc += 1;
                            log!(Category::Scheduler, Verbosity::Debug => w => "WaitUntil woke");
                            w.wake_by_ref();
                        }
                        Task::Notify(_, evt) => {
                            event::EventKeeper::trigger_event(evt);
                            if let Some(waker_vec) = self.wait_on_event.remove(&evt) {
                                woken_prc += waker_vec.len();
                                for w in waker_vec {
                                    log!(Category::Scheduler, Verbosity::Debug => w => "WaitOn woke");
                                    w.wake_by_ref();
                                }
                            } else {
                                log!(Category::Scheduler, Verbosity::Debug
                                    => evt, event::EventKeeper::event_name(evt)
                                    => "Event triggered but no prc waiting on it");
                            }
                        }
                    };
                }
            }
            // Update inflight counter
            shared_api
                .alive_prc
                .fetch_add(woken_prc as isize, Ordering::SeqCst);
            log!(Category::Scheduler, Verbosity::Debug
                    => woken_prc, shared_api
                    => "Scheduler start new delta-cycle");
        }
    }
}

#[cfg(test)]
mod units_tests {

    // TODO extend the unit test coverage
    // #[test]
    // fn test_todo() {
    //     todo!();
    // }
}
