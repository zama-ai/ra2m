//! Define Module abstraction for generic interaction with Hw Module and associated Hw process
//!

use super::*;
use ra2m_sim_output::{log, trace};

use getset::{Getters, Setters};
use regex::Regex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;

/// Module path separator
const PATH_SEP: &str = "::";
/// Module path separator
const IMR_WILDCARD: &str = "*";

/// Common module properties
/// Use to store the module, ClockDomain, PowerDomain, ...
/// Only a placeholder for the moment
#[derive(Debug, Getters, Setters)]
#[getset(get = "pub", set = "pub")]
pub struct Properties {
    uid: usize,
    path: String,
    clock_domain: types::clock::ClockDomain,
    log_filter: log::LogFilter,
    hw_trace: trace::HwTrace,
}

impl Properties {
    pub fn new(path: String, clock_domain: types::clock::ClockDomain) -> Self {
        let uid = MODULE_UID_GEN.fetch_add(1_usize, Ordering::SeqCst);
        Properties {
            uid,
            clock_domain,
            log_filter: Default::default(),
            hw_trace: trace::HwTrace::new(false, &path),
            path,
        }
    }
}

impl Default for Properties {
    fn default() -> Self {
        Self::new(Default::default(), Default::default())
    }
}

/// Common module properties override
/// Enable to derive properties from parents with override of some fields.
#[derive(Debug, Default, Getters, Setters)]
#[getset(get = "pub")]
pub struct PropertiesOverride {
    clock_domain: Option<types::clock::ClockDomain>,
}

/// Static atomic counter used to generate module unique id (uid)
static MODULE_UID_GEN: AtomicUsize = AtomicUsize::new(0);

/// Module trait that every hardware module must implement to integrate themself in the scheduler
/// NB: Module could be nested to describe hierarchical structure. Hierarchical path is composed of
///     path segments separated by `::` pattern  (eg. path_parent::path_child::...)
pub trait Module: Send + Sync {
    /// Get module properties
    fn properties(&self) -> &Arc<Properties>;

    /// Get module Ports
    /// Use to postponned the port binding at the end of elaboration
    /// NB: Provide default implementation for some function while trait is evolving
    ///  => prevent unnecessary break in unit tests
    fn port(&self, name: &str) -> &dyn port::Port;

    /// This function extract sub-modules that match the given path regex
    /// Matching is done only on the first path segment
    /// For extraction of innermost node that match a path pattern, see `inner_match_recurse`
    /// function
    /// A default implementation is provided for flat Module
    fn inner_match(&self, _name: &str) -> Vec<&dyn Module> {
        Vec::new()
    }

    /// Modules without inner modules are considered as tree leaf.
    /// A default implementation is provided for flat Modules
    fn is_leaf(&self) -> bool {
        true
    }

    /// Register process in the scheduler
    fn init(self: Arc<Self>);

    /// Abort module process to drop the associated object
    fn teardown(self: Arc<Self>);
    // TODO extend interface
}

/// Provide a dedicated macro to spawn process on the executor.
/// Spawn the given async function on the underlying async executor. Its aim is to hide executor
/// specificity from the user API as much as possible.
#[macro_export]
macro_rules! spawn_prc {
    ($async_func: expr) => {{
        scheduler::SchedulerAPI::register_new_prc(); // Register within Scheduler
        tokio::spawn(async move {
            $async_func.await; // Run hw_prc
            scheduler::SchedulerAPI::unregister_prc(); // Descheduled on exit
        })
    }};
}

/// Module implementation used to assemble component in a hierarchical manner
/// Use in the top-level architecture to aggregate multiple component in a hierarchical view
/// Also useful for trace/log filtering configuration
pub struct Area {
    props: Arc<Properties>,
    modules: HashMap<String, Arc<dyn Module>>,
    port_map: HashMap<String, (String, String)>,
}

/// AreaError type
/// Describe common error that could occurred with Port /Endpoint management
#[derive(Error, Debug)]
pub enum AreaError {
    #[error("{0} -> In-depth binding not supported yet [{1}]")]
    InDepth(String, String),
    #[error("{0} -> module {1} don't exist")]
    NoModule(String, String),
}

impl Area {
    /// Create new Area with given properties
    pub fn new(props: Properties) -> Self {
        let props = Arc::new(props);
        Self {
            props,
            modules: HashMap::new(),
            port_map: HashMap::new(),
        }
    }
    /// Derive child properties
    /// Extend the path with the given name and override the requested fields
    pub fn child_properties(&self, name: &str, props_ovrd: PropertiesOverride) -> Properties {
        let path = format!("{}{}{}", self.props.path, PATH_SEP, name);
        let mut child_props = Properties::new(path, self.props.clock_domain);
        // Override requested fields
        // TODO this code must be automatically extended with new properties fields
        if let Some(cd) = props_ovrd.clock_domain() {
            child_props.set_clock_domain(*cd);
        }
        child_props
    }

    /// Insert module in the given area
    /// Use last segment of path as module key.
    pub fn insert_module(&mut self, module: Arc<dyn Module>) {
        let key = module
            .properties()
            .path
            .rsplit_once(PATH_SEP)
            .unwrap()
            .1
            .to_owned();
        self.modules.insert(key, module);
    }

    /// Expose inner module::port as top level Area port as name
    pub fn expose_port(&mut self, name: String, module: String, port: String) {
        self.port_map.insert(name, (module, port));
    }

    /// Bind inner module together
    /// WARN: Both inner module must have been already inserted
    pub fn inner_bind(&self, from: &str, to: &str) -> Result<(), anyhow::Error> {
        let from_port = self.get_port_from_str(from)?;
        let to_port = self.get_port_from_str(to)?;
        from_port.bind(to_port.view_as_handle())?;
        Ok(())
    }

    /// Get port from module::name str
    /// Generate meaningful message to user in case of panic
    fn get_port_from_str(&self, mod_port: &str) -> Result<&dyn port::Port, anyhow::Error> {
        let split = mod_port.split(PATH_SEP).collect::<Vec<&str>>();
        if 2 != split.len() {
            return Err(
                AreaError::InDepth(self.props.path().to_string(), mod_port.to_string()).into(),
            );
        }

        if let Some(module) = self.modules.get(split[0]) {
            Ok(module.port(split[1]))
        } else {
            Err(AreaError::NoModule(self.props.path().to_string(), split[0].to_string()).into())
        }
    }
}

impl Area {
    pub fn apply_log_arg(&self, arg: &log::Args) -> usize {
        // Create described log filter
        let filter = log::LogFilter::new(*arg.dflt_verb());
        for (cat, verb) in arg.cat_verb() {
            filter.set(*cat, *verb);
        }

        // Split path and set InnerMatchRecurse mode
        let mut path_seg = arg.path().split(PATH_SEP).collect::<Vec<&str>>();
        let mode = if path_seg[path_seg.len() - 1] == IMR_WILDCARD {
            path_seg.pop();
            ImrMode::Wildcard
        } else {
            ImrMode::ExactMatch
        };

        // Search matching module and replace log filter
        let trgt_modules = inner_match_recurse(vec![self], path_seg.as_slice(), mode);
        for m in trgt_modules.iter() {
            m.properties().log_filter().replace(&filter);
        }
        trgt_modules.len()
    }

    pub fn apply_trace_arg(&self, arg: &trace::Args) -> usize {
        // Split path and set InnerMatchRecurse mode
        let mut path_seg = arg.path().split(PATH_SEP).collect::<Vec<&str>>();
        let mode = if path_seg[path_seg.len() - 1] == IMR_WILDCARD {
            path_seg.pop();
            ImrMode::Wildcard
        } else {
            ImrMode::ExactMatch
        };

        // Search matching module and replace log filter
        let trgt_modules = inner_match_recurse(vec![self], path_seg.as_slice(), mode);
        for m in trgt_modules.iter() {
            let hw_trace = m.properties().hw_trace();
            for (kind, val) in arg.kind_val() {
                hw_trace.update(*kind, *val);
            }
        }
        trgt_modules.len()
    }
}

impl Module for Area {
    fn properties(&self) -> &Arc<Properties> {
        &self.props
    }

    fn port(&self, name: &str) -> &dyn port::Port {
        if let Some((module, port)) = self.port_map.get(name) {
            self.modules[module].port(port)
        } else {
            panic!(
                "{}::{} not exposed externally",
                self.properties().path(),
                name
            );
        }
    }

    fn inner_match(&self, name: &str) -> Vec<&dyn Module> {
        let mut inner = Vec::new();
        let regex = Regex::new(name).unwrap();

        for (k, v) in self.modules.iter() {
            if k.contains(&regex) {
                inner.push(v.as_ref());
            }
        }
        inner
    }

    fn is_leaf(&self) -> bool {
        self.modules.is_empty()
    }

    fn init(self: Arc<Self>) {
        // Init all sub-modules
        // TODO fix init interface, use reference instead of value
        for m in self.modules.values() {
            m.clone().init();
        }
    }

    fn teardown(self: Arc<Self>) {
        // teardown all sub-modules
        for m in self.modules.values() {
            m.clone().teardown();
        }
    }
}

/// InnerMatchRecurse Mode
/// Used to control the end of recursion of inner_match_recurse
/// Could be ended on exact match or recursively wildcard in the hierarchy
#[derive(Clone, Copy)]
enum ImrMode {
    ExactMatch,
    Wildcard,
    WrapUp,
}

/// Recurse in module hierarchy and return list of inner modules leaf that match with the given path
/// NB: Module::inner_match return a list of modules matching on the first path segment only
fn inner_match_recurse<'a>(
    modules: Vec<&'a dyn Module>,
    path_seg: &[&str],
    mode: ImrMode,
) -> Vec<&'a dyn Module> {
    // End of Recursion
    if path_seg.is_empty() {
        match mode {
            ImrMode::ExactMatch => modules,
            ImrMode::Wildcard => {
                // Wrapup with wildcard regex in all remaining level
                inner_match_recurse(modules, &[], ImrMode::WrapUp)
            }
            ImrMode::WrapUp => {
                let mut next_modules = Vec::new();
                for cur_m in modules {
                    next_modules.push(cur_m);
                    if !cur_m.is_leaf() {
                        let next_m = cur_m.inner_match(".*");
                        next_modules.extend(inner_match_recurse(next_m, &[], ImrMode::WrapUp));
                    }
                }
                next_modules
            }
        }
    } else {
        let cur_seg = path_seg[0];
        let mut next_modules = Vec::new();
        for cur_m in modules {
            let next_m = cur_m.inner_match(cur_seg);
            next_modules.extend(inner_match_recurse(next_m, &path_seg[1..], mode));
        }
        next_modules
    }
}

#[cfg(test)]
mod units_tests {
    use super::*;
    use crate::unit::TimeUnit;

    use serial_test::serial;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // Utility Module
    struct ModuleA {
        props: Arc<Properties>,
        port_a: port::MasterPort<usize>,
        log_sender: Mutex<crossbeam::channel::Sender<(usize, time::Tick)>>,
        hw_prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
        cnt: AtomicUsize,
    }

    impl ModuleA {
        pub fn new(
            sender: crossbeam::channel::Sender<(usize, time::Tick)>,
            props: Properties,
        ) -> Self {
            let props = Arc::new(props);
            ModuleA {
                port_a: port::PortNew::new("tx", props.clone(), Some(4), None),
                log_sender: Mutex::new(sender),
                hw_prc: Mutex::new(Vec::new()),
                cnt: AtomicUsize::new(0),
                props,
            }
        }

        async fn prc_delay_for(self: Arc<Self>, tid: usize, delay: time::Tick, nb_iter: usize) {
            let log_s = self.log_sender.lock().unwrap().clone();
            for _i in 0..nb_iter {
                self.cnt.fetch_add(1, Ordering::SeqCst);
                delay::Delay::wait_for(delay).await;
                println!(
                    "{}: prc_delay_for {}::{} fired",
                    time::TimeKeeper::cur_tick(),
                    tid,
                    delay
                );
                log_s.send((tid, time::TimeKeeper::cur_tick())).unwrap();
            }
            println!("Cnt value is {}", self.cnt.load(Ordering::Relaxed));
        }
    }

    impl Module for ModuleA {
        fn properties(&self) -> &Arc<Properties> {
            &self.props
        }

        fn port(&self, name: &str) -> &dyn port::Port {
            match name {
                "port_a" => &self.port_a as &dyn port::Port,
                _ => {
                    panic!(
                        "Invalid port request {} on instance {}",
                        name,
                        self.props.path()
                    );
                }
            }
        }

        fn init(self: Arc<Self>) {
            let mut hw_prc = self.hw_prc.lock().unwrap();
            let asc = self.clone();
            hw_prc.push(spawn_prc!(ModuleA::prc_delay_for(asc, 1, 40, 5)));
            let asc = self.clone();
            hw_prc.push(spawn_prc!(ModuleA::prc_delay_for(asc, 2, 51, 4)));
        }
        fn teardown(self: Arc<Self>) {}
    }

    struct ModuleB {
        props: Arc<Properties>,
        port_b: port::SlavePort<usize>,
        log_sender: Mutex<crossbeam::channel::Sender<(usize, time::Tick)>>,
        hw_prc: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    }

    impl ModuleB {
        pub fn new(
            sender: crossbeam::channel::Sender<(usize, time::Tick)>,
            props: Properties,
        ) -> Self {
            let props = Arc::new(props);
            ModuleB {
                port_b: port::PortNew::new("rx", props.clone(), None, None),
                log_sender: Mutex::new(sender),
                hw_prc: Mutex::new(Vec::new()),
                props,
            }
        }

        async fn prc_delay_until(self: Arc<Self>, tid: usize, delay: time::Tick) {
            let log_s = self.log_sender.lock().unwrap().clone();
            delay::Delay::wait_until(delay).await;
            println!(
                "{}: prc_delay_until {}::{} fired",
                time::TimeKeeper::cur_tick(),
                tid,
                delay
            );
            log_s.send((tid, time::TimeKeeper::cur_tick())).unwrap();
        }
    }

    impl Module for ModuleB {
        fn properties(&self) -> &Arc<Properties> {
            &self.props
        }

        fn port(&self, name: &str) -> &dyn port::Port {
            match name {
                "port_b" => &self.port_b as &dyn port::Port,
                _ => {
                    panic!(
                        "Invalid port request {} on instance {}",
                        name,
                        self.props.path()
                    );
                }
            }
        }

        fn init(self: Arc<Self>) {
            let mut hw_prc = self.hw_prc.lock().unwrap();
            let asc = self.clone();
            hw_prc.push(spawn_prc!(ModuleB::prc_delay_until(asc, 3, 100)));
            let asc = self.clone();
            hw_prc.push(spawn_prc!(ModuleB::prc_delay_until(asc, 4, 202)));
        }
        fn teardown(self: Arc<Self>) {}
    }

    #[tokio::test]
    #[serial]
    async fn test_module() {
        // Create global simulation state and custom scheduler for hardware task
        let mut sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);
        let (log_s, log_r) = crossbeam::channel::unbounded();

        // Instantiate Hw Module
        let mut root = Area::new(Properties::new("root".to_owned(), Default::default()));
        root.insert_module(Arc::new(ModuleA::new(
            log_s.clone(),
            root.child_properties("modA", Default::default()),
        )));
        root.insert_module(Arc::new(ModuleB::new(
            log_s.clone(),
            root.child_properties("modB", Default::default()),
        )));

        // Bind port
        root.inner_bind("modA::port_a", "modB::port_b").unwrap();
        // Init modules
        let root = Arc::new(root);
        root.clone().init();

        // Start scheduler
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

    struct ModuleDummy {
        props: Arc<Properties>,
    }

    impl ModuleDummy {
        pub fn new(props: Properties) -> Self {
            let props = Arc::new(props);
            Self { props }
        }
    }

    impl Module for ModuleDummy {
        fn properties(&self) -> &Arc<Properties> {
            &self.props
        }

        fn port(&self, name: &str) -> &dyn port::Port {
            panic!(
                "Invalid port request {} on instance {}",
                name,
                self.props.path()
            );
        }

        fn init(self: Arc<Self>) {}
        fn teardown(self: Arc<Self>) {}
    }

    #[tokio::test]
    #[serial]
    async fn test_area_recurse() {
        // Create global simulation state and custom scheduler for hardware task
        let _sched = init_simulation(0, 1.ps(), time::TimingMode::LT, DFLT_HW_PROCESS);

        // Create fake hierarchy
        let level1 = vec!["ABC", "DEF", "GHI", "JKL", "MNO", "PQR", "STU", "VWX", "YZ"];
        let level2 = vec!["abc", "def", "ghi", "jkl", "mno", "pqr", "stu", "vwx", "yz"];

        let mut root = Area::new(Properties::new("root".to_owned(), Default::default()));
        for l1 in level1.iter() {
            let name = format!("l1_{l1}");
            let mut lvl1 = Area::new(root.child_properties(&name, Default::default()));

            for l2 in level2.iter() {
                let name = format!("l2_{l2}");
                let mut lvl2 = Area::new(lvl1.child_properties(&name, Default::default()));

                for leaf in 0..20 {
                    let name = format!("leaf_{leaf}");
                    lvl2.insert_module(Arc::new(ModuleDummy::new(
                        lvl2.child_properties(&name, Default::default()),
                    )));
                }
                lvl1.insert_module(Arc::new(lvl2));
            }
            root.insert_module(Arc::new(lvl1));
        }

        // Init nested module tree
        let root = Arc::new(root);
        root.clone().init();

        let test_path = [
            ("l1_ABC::.*::leaf_1$", 9),
            ("l1_.*::l2_def::leaf_[1,2]$", 18),
            ("l1_.*::l2_def::leaf_[1,2]*$", 36),
            ("l1_PQR::l2_mno::*", 21),
            ("l1_PQR::l2_aqr::*", 0),
            ("l1_PQR::.*", 9),
            ("l1_(ABC|VWX)::.*", 18),
            ("l1_(ABC|VWX)", 2),
            ("l1_GH.::*", 190),
        ];

        // Start regex match in the tree and assert number of matching modules
        for (path, match_len) in test_path {
            // Split path and set InnerMatchRecurse mode
            let mut path_seg = path.split(PATH_SEP).collect::<Vec<&str>>();
            let mode = if path_seg[path_seg.len() - 1] == IMR_WILDCARD {
                path_seg.pop();
                ImrMode::Wildcard
            } else {
                ImrMode::ExactMatch
            };
            // Search matching module and display path and assert count
            let match_modules = inner_match_recurse(vec![root.as_ref()], path_seg.as_slice(), mode);
            println!("Match on regex: {path}[{match_len}]");
            for m in match_modules.iter() {
                println!("-> {}", m.properties().path());
            }
            assert_eq!(match_len, match_modules.len());
            println!();
        }
    }
}
