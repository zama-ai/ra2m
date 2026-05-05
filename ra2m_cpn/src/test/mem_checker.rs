//! Memory Checker
//! Modelize behavioral memory
//! It received tuple of Req/Resp through mpsc channel and assert their content against the
//! embedded behavioral memory.
//!

use protocol::{
    addr::{Addr, Pattern},
    membus::Command,
};
use ra2m_sim::prelude::*;

use getset::{Getters, MutGetters, Setters};
use std::io::Write;
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Checker structure for MemBus,
/// Only contains subset of MemBus field data is held by an plain vector
#[derive(Debug, Getters, MutGetters, Setters)]
#[getset(get = "pub")]
pub struct MemBusChecker {
    cmd: Command,
    addr: Addr,
    pattern: Pattern,
    #[getset(get = "pub", get_mut = "pub", set = "pub")]
    data: Vec<u8>,
}

impl MemBusChecker {
    pub fn new(cmd: Command, addr: Addr, pattern: Pattern) -> Self {
        MemBusChecker {
            cmd,
            addr,
            pattern,
            data: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Getters)]
#[getset(get = "pub")]
pub struct MemoryCheckerParams {
    pub offset: usize,
    pub size: unit::Data,
    pub binfile: Option<String>,
}

pub struct MemoryChecker {
    params: MemoryCheckerParams,
    check_rx: mpsc::Receiver<MemBusChecker>,
    memory: Mutex<Vec<u8>>,
}

impl MemoryChecker {
    pub fn start(
        params: MemoryCheckerParams,
        rx: mpsc::Receiver<MemBusChecker>,
    ) -> tokio::task::JoinHandle<()> {
        let size: usize = (&params.size).into();

        // Init content of memory from binary file or default init
        let mut mem;
        if let Some(binfile) = &params.binfile {
            mem = std::fs::read(binfile).unwrap();
        } else {
            mem = Vec::with_capacity(size);
        }
        mem.resize(size, 0);

        // Create checker instance
        let mem_check = MemoryChecker {
            params,
            check_rx: rx,
            memory: Mutex::new(mem),
        };

        // Spawn async check function and consume checker
        tokio::spawn(async move {
            Self::check_rx_stream(mem_check).await;
        })
    }

    async fn check_rx_stream(mut self) {
        loop {
            // Get next req_resp payload
            if let Some(req) = self.check_rx.recv().await {
                log!(log::Category::Test, log::Verbosity::Trace => req);
                // Align memory access with offset and apply action in memory
                let addr = match req.addr() {
                    Addr::Phys(t) => *t - self.params.offset,
                    Addr::PhysVirt(t, _) => *t - self.params.offset,
                    _ => {
                        panic!("Received request with invalid Addr {:?}.", req.addr());
                    }
                };

                match req.pattern() {
                    Pattern::Simple(d) => {
                        match req.cmd() {
                            Command::Read => {
                                // Get content of behavioral memory and assert resp content
                                let len: usize = d.into();
                                let mem_slice = &self.memory.lock().unwrap()[addr..(addr + len)];
                                assert_eq!(
                                    mem_slice,
                                    req.data().as_slice(),
                                    "Faulty access {req:?}",
                                );
                            }
                            Command::Write => {
                                // Apply Write into behavioral memory
                                let len = d.into();
                                let req_slice = req.data().as_slice();
                                assert_eq!(req_slice.len(), len);
                                let mut mem = self.memory.lock().unwrap();
                                mem[addr..(addr + len)].copy_from_slice(&req_slice[..len]);
                            }
                            _ => {
                                panic!("Received unsupported command {:?}", req.cmd());
                            }
                        };
                    }
                    _ => {
                        panic!("Received unsupported pattern {:?}", req.pattern());
                    }
                }
            } else {
                panic!("Attempt to recv check tuple across an closed channel")
            }
        }
    }
}
