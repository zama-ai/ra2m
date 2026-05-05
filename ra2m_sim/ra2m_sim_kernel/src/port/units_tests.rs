use super::*;
use crate::unit::TimeUnit;

use serial_test::serial;
use std::sync::Arc;

/// Dummy traffic generator send packet on a Dummy loopback and assert return values
struct DummyTraffic {
    p_out: MasterPort<usize>,
    p_in: SlavePort<usize>,
}

impl DummyTraffic {
    pub fn new(props: module::Properties, pkt_inflight: usize) -> Self {
        let props = Arc::new(props);
        DummyTraffic {
            p_out: PortNew::new("tx", props.clone(), Some(pkt_inflight), None),
            p_in: PortNew::new("rx", props, None, None),
        }
    }

    async fn generator(self: Arc<Self>, port_delay: usize, task_delay: usize, iter: usize) {
        for i in 0..iter {
            // Create payload
            let pld = i;
            // Wrap it in packet
            let pkt = Packet::wrap_payload(
                pld,
                PacketOptions {
                    delay: port_delay,
                    ..Default::default()
                },
            );

            // lock the port and send
            let mut p_out = self.p_out.0.lock().await;
            p_out.send(pkt).await;
            println!(
                "********************************************************************************"
            );
            println!("{}:: Traffic Send {}::[{}]", cur_tick(), i, port_delay);

            // Dummy computation
            std::thread::sleep(std::time::Duration::from_millis(10));

            // Dummy simulation delay
            delay::Delay::wait_for(task_delay).await;

            let mut p_in = self.p_in.0.lock().await;
            let mut pkt = p_in.recv().await;
            pkt.handle_delay().await;
            println!("{}:: Traffic Recv {:?}::[{}]", cur_tick(), pkt, i);
            println!(
                "********************************************************************************"
            );
        }
    }

    pub fn init(self: &Arc<Self>, iter: usize) {
        let asc = self.clone();
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        tokio::spawn(async move {
            DummyTraffic::generator(asc, 2, 20, iter).await; // Run hw prc
            scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit
        });
    }
}

struct DummyLoopback {
    p_in: SlavePort<usize>,
    p_out: MasterPort<usize>,
}

impl DummyLoopback {
    pub fn new(props: module::Properties, pkt_inflight: usize) -> Self {
        let props = Arc::new(props);
        DummyLoopback {
            p_in: PortNew::new("rx", props.clone(), None, None),
            p_out: PortNew::new("tx", props, Some(pkt_inflight), None),
        }
    }

    async fn loopback(self: Arc<Self>, delay: usize) {
        loop {
            let mut p_in = self.p_in.0.lock().await;
            let mut pkt = p_in.recv().await;
            println!("{}:: Loopback {:?}", cur_tick(), pkt);

            // Dummy task delay
            pkt.account_delay(Some(delay)).await;

            let mut p_out = self.p_out.0.lock().await;
            p_out.send(pkt).await;
        }
    }

    pub fn init(self: &Arc<Self>) {
        let asc = self.clone();
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        tokio::spawn(async move {
            DummyLoopback::loopback(asc, 10).await; // Run hw prc
            scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit
        });
    }
}

#[tokio::test]
#[serial]
async fn test_port() {
    // Create global simulation state and custom scheduler for hardware task
    let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

    // Instantiate Dummy Hw
    let pkt_inflight = 6;
    let traffic = Arc::new(DummyTraffic::new(
        module::Properties::new("DummyTraffic".to_owned(), Default::default()),
        pkt_inflight,
    ));
    let loopback = Arc::new(DummyLoopback::new(
        module::Properties::new("DummyLoopback".to_owned(), Default::default()),
        pkt_inflight,
    ));

    // Bind port
    traffic.p_in.bind(loopback.p_out.view_as_handle()).unwrap();
    println!(" Bind success");

    traffic.p_out.bind(loopback.p_in.view_as_handle()).unwrap();
    println!(" Bind success");

    // Init Dummy process
    traffic.init(40);
    loopback.init();

    // Start scheduler
    sched.simulate(400).await;
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct Msg {
    kind: usize,
    data: [u64; 20],
}

/// Dummy traffic generator send packet on a Dummy loopback and assert return values
struct DummyRRTraffic {
    p_rr: ReqRespPort<Msg>,
}

impl DummyRRTraffic {
    pub fn new(props: module::Properties, pkt_inflight: usize) -> Self {
        let props = Arc::new(props);
        Self {
            p_rr: ReqRespPort::new("rr", props, Some(pkt_inflight), None),
        }
    }

    async fn generator(self: Arc<Self>, port_delay: usize, task_delay: usize, iter: usize) {
        for i in 0..iter {
            // Create packet
            let pkt = Packet::wrap_payload(
                Msg {
                    kind: i,
                    data: [i as u64; 20],
                },
                Default::default(),
            );

            // lock the port and send
            let mut rr_tx = self.p_rr.tx().0.lock().await;
            rr_tx.send(pkt).await;
            drop(rr_tx);
            println!(
                "********************************************************************************"
            );
            println!("{}:: RRTraffic Send {}::[{}]", cur_tick(), i, port_delay);

            // Dummy computation
            std::thread::sleep(std::time::Duration::from_millis(10));

            // Dummy simulation delay
            delay::Delay::wait_for(task_delay).await;

            // lock the port and wait an recv
            let mut rr_rx = self.p_rr.rx().0.lock().await;
            let mut pkt = rr_rx.recv().await;
            drop(rr_rx);
            pkt.handle_delay().await;
            println!(
                "{}:: Traffic Recv {:?}::[{}] -> {:?}",
                cur_tick(),
                pkt,
                i,
                pkt.payload()
            );
            println!(
                "********************************************************************************"
            );
        }
    }

    pub fn init(self: &Arc<Self>, iter: usize) {
        let asc = self.clone();
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        tokio::spawn(async move {
            DummyRRTraffic::generator(asc, 2, 20, iter).await; // Run hw prc
            scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit
        });
    }
}

struct DummyRRLoopback {
    p_rr: ReqRespPort<Msg>,
}

impl DummyRRLoopback {
    pub fn new(props: module::Properties) -> Self {
        let props = Arc::new(props);
        Self {
            p_rr: ReqRespPort::new("rr", props, None, None),
        }
    }

    async fn loopback(self: Arc<Self>, delay: usize) {
        loop {
            let mut rr_rx = self.p_rr.rx().lock().await;
            let mut pkt = rr_rx.recv().await;
            drop(rr_rx);
            println!("{}:: Loopback {:?} => {:?}", cur_tick(), pkt, pkt.payload());

            // Dummy task delay
            pkt.account_delay(Some(delay)).await;

            let mut rr_tx = self.p_rr.tx().0.lock().await;
            rr_tx.send(pkt).await;
            drop(rr_tx);
        }
    }

    pub fn init(self: &Arc<Self>) {
        let asc = self.clone();
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        tokio::spawn(async move {
            DummyRRLoopback::loopback(asc, 10).await; // Run hw prc
            scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit
        });
    }
}

#[tokio::test]
#[serial]
async fn test_rrport() {
    // Create global simulation state and custom scheduler for hardware task
    let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

    // Instantiate Dummy Hw
    let pkt_inflight = 6;
    let traffic = Arc::new(DummyRRTraffic::new(
        module::Properties::new("DummyRRTraffic".to_owned(), Default::default()),
        pkt_inflight,
    ));
    let loopback = Arc::new(DummyRRLoopback::new(module::Properties::new(
        "DummyRRLoopback".to_owned(),
        Default::default(),
    )));

    // Bind port
    traffic.p_rr.bind((&loopback.p_rr).into()).unwrap();

    // Init Dummy process
    traffic.init(40);
    loopback.init();

    // Start scheduler
    sched.simulate(400).await;
}
