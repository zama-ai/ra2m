//! Traffic Generator
//! Generate random traffic over ReqRespPort and forward req/resp to a checker through mpsc channel
//! NB: Only generate Simple Pattern access ATM

use protocol::{
    addr::{Addr, Pattern},
    membus::{Command, MemBus},
};
use ra2m_sim::prelude::*;

use super::*;

use getset::CopyGetters;
use rand::{RngExt, SeedableRng, rngs::StdRng};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Clone, CopyGetters)]
#[getset(get_copy = "pub")]
pub struct TrafficGenParams {
    pub addr_range: (usize, unit::Data),
    pub addr_chunk: unit::Data,
    pub data_range: (unit::Data, unit::Data),
    pub read_write_weight: (usize, usize),
    pub inflight_req: usize,
    pub rng_seed: Option<u64>,
}

#[derive(Module)]
pub struct TrafficGen {
    params: TrafficGenParams,
    props: Arc<module::Properties>,
    #[port]
    req_port: port::ReqRespPort<MemBus>,
    prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    checker_tx: mpsc::Sender<MemBusChecker>,
}

#[default_teardown]
impl TrafficGen {
    pub fn new(
        params: TrafficGenParams,
        props: module::Properties,
        checker_tx: mpsc::Sender<MemBusChecker>,
    ) -> Self {
        let props = Arc::new(props);
        TrafficGen {
            req_port: port::ReqRespPort::new("req", props.clone(), Some(params.inflight_req), None),
            prc: Mutex::new(Vec::new()),
            checker_tx,
            params,
            props,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        // Initialize internal memory array

        // Start prc_rep_answer
        let mut prc = self.prc.lock().unwrap();
        let asc = self.clone();
        prc.push(spawn_prc!(Self::gen_traffic(asc)));
    }

    async fn gen_traffic(self: Arc<Self>) {
        let mut rng: StdRng = if let Some(seed) = self.params.rng_seed {
            SeedableRng::seed_from_u64(seed)
        } else {
            rand::make_rng()
        };

        let (addr_low, addr_range_size) = self.params.addr_range();
        let addr_high = addr_low + usize::from(addr_range_size);

        let (data_low, data_high) = self.params.data_range();
        let len_low = usize::from(data_low);
        let len_high = usize::from(data_high);

        let (read_w, write_w) = self.params.read_write_weight();
        let cmd_w = read_w + write_w;

        loop {
            // Draw randomness
            let phys_addr = rng.random_range(addr_low..addr_high);
            // Prevent chunk boundary crossing
            // => max access size = chunk_size - ofst in chunk
            let len_max = usize::from(self.params.addr_chunk)
                - (phys_addr % usize::from(self.params.addr_chunk));
            if len_low > len_max {
                continue;
            }; // Couldn't match user request
            let len = rng.random_range(len_low..std::cmp::min(len_high, len_max));

            // Format request field based on drawn randomness
            let addr = Addr::Phys(phys_addr);
            let pattern = Pattern::Simple(len.Byte());
            let cmd_rand = rng.random_range(0..cmd_w);
            let (cmd, wval) = if cmd_rand < read_w {
                (Command::Read, None)
            } else {
                let mut wval = Vec::with_capacity(len);
                for _ in 0..len {
                    let d: u8 = rng.random();
                    wval.push(d);
                }
                (Command::Write, Some(wval))
            };

            // Forge the request and the checker
            let pkt = MemBus::new_wrapped(
                self.props.uid(),
                cmd,
                addr,
                pattern,
                wval.as_ref().map(Vec::as_ref),
                None,
            );
            let mut checker_pkt = MemBusChecker::new(cmd, addr, pattern);

            if let Some(data) = wval {
                checker_pkt.set_data(data);
            }
            // Send it over ReqResp port
            // TODO handle multiple outstanding request
            self.req_port
                .tx()
                .send_pkt(pkt)
                .await
                .expect("Faulty Tx access");

            // Wait response
            let mut pkt = self
                .req_port
                .rx()
                .wait_pkt_ep(None)
                .await
                .expect("Faulty Rx response");

            // View protocol layer and update checker if needed
            let resp = pkt.payload_mut();
            if Command::Read == cmd {
                checker_pkt
                    .data_mut()
                    .extend_from_slice(resp.data().as_slice());
            }
            self.checker_tx.send(checker_pkt).await.unwrap();
        }
    }
}
