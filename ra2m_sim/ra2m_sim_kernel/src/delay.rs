//! Let Hw Process to wait for a given amount of simulated time (Delay)
//! Those Delay could be expressed in term of relative tick (relative to the current simulation
//! tick) or absolute tick (relative to simulation tick 0)
//!
//! Hw process delay rely on async runtime and custom Future implementation

use super::*;

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Delay structure that enable an async function to wait for a given number of
/// Tick in the simulated time
#[derive(Debug)]
pub struct Delay {
    wake_at: time::Tick,
    forged_at: time::Tick,
    order_pushed: bool,
}

impl Delay {
    pub fn wait_for(delay: time::Tick) -> Self {
        let forged_at = time::TimeKeeper::cur_tick();
        let wake_at = forged_at + delay;

        Delay {
            wake_at,
            forged_at,
            order_pushed: false,
        }
    }

    pub fn wait_until(tick: time::Tick) -> Self {
        let forged_at = time::TimeKeeper::cur_tick();
        Delay {
            wake_at: tick,
            forged_at,
            order_pushed: false,
        }
    }
}

impl Future for Delay {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let ltick = time::TimeKeeper::cur_tick();
        // Ready path
        if ltick >= self.wake_at {
            Poll::Ready(())
        } else {
            // Register wait until event in the scheduler
            assert!(
                ltick == self.forged_at,
                "Delay not directly awaited: await@{} forged@{}",
                ltick,
                self.forged_at
            );
            if !self.order_pushed {
                // Prevent deadlock on unneeded poll
                let order = scheduler::Order::WaitUntil(self.wake_at, cx.waker().clone());
                scheduler::SchedulerAPI::push_order(order);
                self.order_pushed = true;
            }
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod units_tests {
    use super::*;
    use crate::unit::TimeUnit;

    use serial_test::serial;

    // Utility functions
    async fn prc_delay_for(
        tid: usize,
        delay: time::Tick,
        nb_iter: usize,
        log_sender: crossbeam::channel::Sender<(usize, time::Tick)>,
    ) {
        for _i in 0..nb_iter {
            delay::Delay::wait_for(delay).await;
            println!(
                "{}: prc_delay_for {}::{} fired",
                time::TimeKeeper::cur_tick(),
                tid,
                delay
            );
            log_sender
                .send((tid, time::TimeKeeper::cur_tick()))
                .unwrap();
        }
        scheduler::SchedulerAPI::unregister_prc();
    }

    async fn prc_delay_until(
        tid: usize,
        delay: time::Tick,
        log_sender: crossbeam::channel::Sender<(usize, time::Tick)>,
    ) {
        delay::Delay::wait_until(delay).await;
        println!(
            "{}: prc_delay_until {}::{} fired",
            time::TimeKeeper::cur_tick(),
            tid,
            delay
        );
        log_sender
            .send((tid, time::TimeKeeper::cur_tick()))
            .unwrap();
        scheduler::SchedulerAPI::unregister_prc();
    }

    #[tokio::test]
    #[serial]
    async fn test_delay() {
        // Create global simulation state and custom scheduler for hardware task
        let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);
        let (log_s, log_r) = crossbeam::channel::unbounded();

        // Spawned hardware tasks and join
        let mut hw_prc = Vec::new();
        let log_sender = log_s.clone();
        hw_prc.push(tokio::spawn(async move {
            prc_delay_for(1, 40, 5, log_sender).await;
        }));
        let log_sender = log_s.clone();
        hw_prc.push(tokio::spawn(async move {
            prc_delay_for(2, 51, 4, log_sender).await;
        }));
        let log_sender = log_s.clone();
        hw_prc.push(tokio::spawn(async move {
            prc_delay_until(3, 100, log_sender).await;
        }));
        let log_sender = log_s.clone();
        hw_prc.push(tokio::spawn(async move {
            prc_delay_until(4, 202, log_sender).await;
        }));

        // Register added hw_process as inflight
        for _i in 0..hw_prc.len() {
            scheduler::SchedulerAPI::register_new_prc();
        }
        sched.simulate(400).await;

        // Read order and check it
        let mut prc_log = Vec::new();
        while let Ok((tid, tick)) = log_r.try_recv() {
            prc_log.push((tid, tick));
        }
        assert_eq!(
            prc_log,
            vec![
                (1, 40),
                (2, 51),
                (1, 80),
                (3, 100),
                (2, 102),
                (1, 120),
                (2, 153),
                (1, 160),
                (1, 200),
                (4, 202),
                (2, 204)
            ]
        );
    }
}
