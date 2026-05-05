use log::{Category, LogFilter, Verbosity};
use ra2m_sim::prelude::*;

use serial_test::serial;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Define Golden log file based on lowest log verbosity
/// i.e. without debug feature, all Debug/Trace messages are disabled
#[cfg(feature = "debug")]
pub const GOLDEN_PATH: &str = "tests/text_logger_golden_debug.log";
#[cfg(not(feature = "debug"))]
pub const GOLDEN_PATH: &str = "tests/text_logger_golden.log";

#[derive(Module)]
struct LogModule {
    props: Arc<module::Properties>,
    hw_prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    a: usize,
    b: i32,
}

#[default_teardown]
impl LogModule {
    pub fn new() -> Self {
        let props = Arc::new(module::Properties::new(
            "root.logger.AwesomeLoggerName".to_owned(),
            Default::default(),
        ));

        LogModule {
            props,
            hw_prc: Mutex::new(Vec::new()),
            a: 0,
            b: -8,
        }
    }

    #[init]
    fn _init(self: Arc<Self>) {
        let mut hw_prc = self.hw_prc.lock().unwrap();
        let asc = self.clone();
        hw_prc.push(spawn_prc!(LogModule::prc_mlog_n_delay(asc, 10)));
        let asc = self.clone();
        hw_prc.push(spawn_prc!(LogModule::prc_glog_n_delay(asc, 10)));
    }

    async fn prc_glog_n_delay(self: Arc<Self>, delay: time::Tick) {
        log!(Category::Port, Verbosity::Error
            => delay
            => "Should not appear in log file due to global disable");

        delay::Delay::wait_for(30).await;

        // Test default arguments handling in macro
        log!(,=> );
        log!(,=> /* empty variable list */
            => "Global: All default with no vars and a custom user message");

        let mut dflt_cat = true;
        let mut dflt_verb = true;

        log!(,=> dflt_cat, dflt_verb);
        log!(,=> dflt_cat, dflt_verb
            => "Global: With custom user message");

        dflt_cat = false;
        log!(Category::Port,=> dflt_cat, dflt_verb);
        log!(Category::Port,=> dflt_cat, dflt_verb
            => "Global: With custom user message");

        dflt_cat = true;
        dflt_verb = false;
        log!(,Verbosity::Error=> dflt_cat, dflt_verb);
        log!(,Verbosity::Error=> dflt_cat, dflt_verb
            => "Global: With custom user message");

        // variables to be logged
        let val_usize = 264_usize;
        let val_str = "Simple &str";
        let val_vec = vec![1, 2, 3, 4, 5];
        for iter in 0..5 {
            delay::Delay::wait_for(delay).await;
            log!(Category::Port, Verbosity::Error
                => iter, val_usize, val_str, val_vec
                => "Global scope filter");
        }
    }

    async fn prc_mlog_n_delay(self: Arc<Self>, delay: time::Tick) {
        log!(Category::Port, Verbosity::Error
            => delay
            => "Should not appear in log file due to global disable");

        delay::Delay::wait_for(35).await;

        // Test default arguments handling in macro
        log!(|self|,=> );
        log!(|self|,=> /* empty variable list */
            => "Local: All default with no vars and a custom user message");

        let mut dflt_cat = true;
        let mut dflt_verb = true;

        log!(|self|,=> dflt_cat, dflt_verb);
        log!(|self|,=> dflt_cat, dflt_verb
            => "Local: With custom user message");

        dflt_cat = false;
        log!(|self| Category::Port,=> dflt_cat, dflt_verb);
        log!(|self| Category::Port,=> dflt_cat, dflt_verb
            => "Local: With custom user message");

        dflt_cat = true;
        dflt_verb = false;
        log!(|self| ,Verbosity::Error=> dflt_cat, dflt_verb);
        log!(|self| ,Verbosity::Error=> dflt_cat, dflt_verb
            => "Local: With custom user message");

        // variables to be logged
        for iter in 0..5 {
            delay::Delay::wait_for(delay).await;
            log!(|self| Category::Own, Verbosity::Debug
                => iter
                => "Module scope filter");
        }

        // member to be logged
        delay::Delay::wait_for(delay).await;
        log!(|self| Category::Own, Verbosity::Error
            => self.a, self.b
            => "log member value with self.name syntax");
    }
}

#[tokio::main]
#[test] // NB: Use of expected macro #[tokio::test] failed due to nested crates
#[serial]
async fn test_logger() {
    let log_path = "/tmp/ra2m/ra2m_sim/integration_tests/test_logger";
    Output::init(log_path);

    // Create global simulation state and custom scheduler for hardware task
    let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

    // Instantiate Hw Module
    let hw_module: Vec<Arc<dyn module::Module>> = vec![Arc::new(LogModule::new())];

    // For each module, display proterties and spawn associated task
    for m in &hw_module {
        println!("Module properties: {:?}.", m.properties());
        m.clone().init();
    }

    // Start scheduler
    sched.simulate(20).await;

    // Turn log on and change verbosity to Debug for all cat
    hw_module[0]
        .properties()
        .log_filter()
        .set(Category::Own, Verbosity::Debug);
    Output::log_update(true, Some(LogFilter::new(Verbosity::Info)));

    sched.simulate(400).await;

    // Check log file content
    let expanded_log_path = Path::new(log_path).join("latest_output").join("ra2m.log");
    let log_lines = match File::open(&expanded_log_path) {
        Err(why) => panic!("couldn't open {}: {why}", expanded_log_path.display()),
        Ok(file) => io::BufReader::new(file).lines(),
    };
    let log: Vec<_> = log_lines.collect();

    let golden_path = Path::new(GOLDEN_PATH);
    let golden_lines = match File::open(golden_path) {
        Err(why) => panic!("couldn't open {}: {why}", golden_path.display()),
        Ok(file) => io::BufReader::new(file).lines(),
    };
    let golden: Vec<_> = golden_lines.collect();

    assert_eq!(log.len(), golden.len());

    for (i, (log, golden)) in log.iter().zip(golden.iter()).enumerate() {
        match (log, golden) {
            (Ok(l), Ok(g)) => {
                assert_eq!(l, g, "Line mismatch at {i}")
            }
            _ => {
                panic!("Line error at {i}")
            }
        }
    }
}
