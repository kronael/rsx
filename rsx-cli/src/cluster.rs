use crate::catalog;
use crate::output::Failure;
use crate::output::Outcome;
use crate::plan;
use crate::plan::AuthSetup;
use crate::plan::MakerTuning;
use crate::plan::Options;
use crate::plan::Plan;
use crate::plan::Process;
use crate::plan::AUTH_NAME;
use crate::plan::MAKER_CONFIG_FILE;
use crate::plan::MAKER_NAME;
use crate::probe;
use crate::probe::Probe;
use crate::procs;
use crate::procs::Record;
use crate::procs::State;
use crate::procs::Stopped;
use crate::procs::Store;
use crate::runtime_utils::block_on;
use crate::scenario;
use crate::scenario::Scenario;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;
use tokio::time::sleep;
use tokio::time::Instant;

#[cfg(test)]
#[path = "cluster_test.rs"]
mod cluster_test;

const STOP_GRACE: Duration = Duration::from_secs(5);
const SPAWN_GAP: Duration = Duration::from_millis(100);
const READY_POLL: Duration = Duration::from_millis(200);
const SIDECAR_GRACE: Duration = Duration::from_millis(1500);
const LOG_TAIL_LINES: usize = 5;

/// A process left out of an action, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

impl Skipped {
    fn new(name: &str, reason: impl ToString) -> Self {
        Self {
            name: name.to_string(),
            reason: reason.to_string(),
        }
    }
}

/// What `up`, `down`, `restart` and `reset` did. `waited_ms` is how long `up` or
/// `restart` waited for the processes it started.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub waited_ms: Option<u64>,
    /// What `reset` wiped: `wal`, `tips`, `risk-state`.
    pub cleared: Vec<String>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        let mut report = json!({
            "started": self.started,
            "stopped": self.stopped,
            "skipped": self.skipped.iter().map(|skipped| {
                json!({ "name": skipped.name, "reason": skipped.reason })
            }).collect::<Vec<_>>(),
        });
        if let (Some(waited_ms), Some(object)) = (self.waited_ms, report.as_object_mut()) {
            object.insert("waited_ms".to_string(), json!(waited_ms));
        }
        if let (false, Some(object)) = (self.cleared.is_empty(), report.as_object_mut()) {
            object.insert("cleared".to_string(), json!(self.cleared));
        }
        report
    }

    pub fn to_text(&self) -> String {
        let mut lines = Vec::new();
        if !self.started.is_empty() {
            lines.push(format!("started: {}", self.started.join(", ")));
        }
        if !self.stopped.is_empty() {
            lines.push(format!("stopped: {}", self.stopped.join(", ")));
        }
        for skipped in &self.skipped {
            lines.push(format!("skipped {}: {}", skipped.name, skipped.reason));
        }
        if !self.cleared.is_empty() {
            lines.push(format!("cleared: {}", self.cleared.join(", ")));
        }
        if let Some(waited_ms) = self.waited_ms {
            lines.push(format!("waited {:.1} s", waited_ms as f64 / 1000.0));
        }
        lines.join("\n")
    }
}

/// Whether a process is up, gone, or never started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Exited,
    Absent,
}

impl ProcessState {
    pub fn label(self) -> &'static str {
        match self {
            ProcessState::Running => "running",
            ProcessState::Exited => "exited",
            ProcessState::Absent => "absent",
        }
    }
}

/// One row of `ps`.
#[derive(Debug, PartialEq, Eq)]
pub struct Status {
    pub name: String,
    pub pid: Option<u32>,
    pub state: ProcessState,
    pub started_ms: Option<u64>,
    pub log: String,
}

impl Status {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "pid": self.pid,
            "state": self.state.label(),
            "started_ms": self.started_ms,
            "log": self.log,
        })
    }
}

/// The settings every cluster action shares.
#[derive(Clone)]
pub struct Cluster {
    pub root: PathBuf,
    pub database_url: String,
    pub jwt_secret: String,
    pub release: bool,
    pub pin_cores: bool,
    pub no_maker: bool,
}

struct Assembled {
    plan: Plan,
    skipped: Vec<Skipped>,
}

struct Prepared {
    spawn: Vec<Process>,
    report: Report,
}

fn is_sidecar(name: &str) -> bool {
    name == MAKER_NAME || name == AUTH_NAME
}

fn probe_for(process: &Process) -> Probe {
    if process.is_replica() {
        Probe::Live
    } else {
        Probe::Ready
    }
}

/// The processes of `plan` named in `only`, all of them when `only` is empty;
/// an unknown name is a `Misuse` failure listing the plan's names.
fn select_processes<'a>(plan: &'a Plan, only: &[String]) -> Result<Vec<&'a Process>, Failure> {
    for name in only {
        if plan.process(name).is_none() {
            let known: Vec<&str> = plan
                .processes
                .iter()
                .map(|process| process.name.as_str())
                .collect();
            return Err(Failure::misuse(format!(
                "{name:?} is not in scenario {} (processes: {})",
                plan.scenario,
                known.join(", ")
            )));
        }
    }
    Ok(plan
        .processes
        .iter()
        .filter(|process| only.is_empty() || only.contains(&process.name))
        .collect())
}

async fn shutdown_signal() {
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

impl Cluster {
    fn store(&self) -> Store {
        Store::new(&self.root)
    }

    fn options(&self, maker: Option<MakerTuning>, auth: Option<AuthSetup>) -> Options {
        Options {
            database_url: self.database_url.clone(),
            release: self.release,
            pin_cores: self.pin_cores,
            jwt_secret: self.jwt_secret.clone(),
            maker,
            auth,
        }
    }

    fn read_maker_tuning(&self) -> MakerTuning {
        match fs::read_to_string(self.root.join(MAKER_CONFIG_FILE)) {
            Ok(text) => MakerTuning::from_json(&text),
            Err(_) => MakerTuning::default(),
        }
    }

    fn read_auth_setup(&self) -> Result<AuthSetup, String> {
        if !self.root.join("rsx-auth/.venv/bin/python").exists() {
            return Err(
                "rsx-auth/.venv/bin/python is missing (run uv sync in rsx-auth)".to_string(),
            );
        }
        let is_set = |key: &str| std::env::var(key).is_ok_and(|value| !value.is_empty());
        if !is_set("RSX_AUTH_GITHUB_CLIENT_ID") || !is_set("RSX_AUTH_GITHUB_CLIENT_SECRET") {
            return Err(
                "not configured (RSX_AUTH_GITHUB_CLIENT_ID and RSX_AUTH_GITHUB_CLIENT_SECRET)"
                    .to_string(),
            );
        }
        let mut setup = AuthSetup::default();
        let read = |key: &str, slot: &mut String| {
            if let Ok(value) = std::env::var(key) {
                *slot = value;
            }
        };
        read("RSX_AUTH_LISTEN", &mut setup.listen);
        read("RSX_AUTH_REDIRECT_URI", &mut setup.redirect_uri);
        read("RSX_AUTH_TRADE_UI_URL", &mut setup.trade_ui_url);
        read("RSX_AUTH_STARTER_COLLATERAL", &mut setup.starter_collateral);
        read("RSX_AUTH_JWT_TTL_S", &mut setup.jwt_ttl_s);
        Ok(setup)
    }

    async fn assemble(&self, scenario: &Scenario) -> Result<Assembled, Failure> {
        let catalog = catalog::load(&self.database_url).await?;
        let symbols = plan::select_symbols(scenario, &catalog)?;
        let mut skipped = Vec::new();
        let maker = if self.no_maker {
            skipped.push(Skipped::new(MAKER_NAME, "disabled by --no-maker"));
            None
        } else {
            Some(self.read_maker_tuning())
        };
        let auth = match self.read_auth_setup() {
            Ok(setup) => Some(setup),
            Err(reason) => {
                skipped.push(Skipped::new(AUTH_NAME, reason));
                None
            }
        };
        let plan = plan::build(scenario, &symbols, &self.options(maker, auth))?;
        Ok(Assembled { plan, skipped })
    }

    /// Prints the plan of `scenario_name`: the processes, their env and ports.
    pub fn plan(&self, scenario_name: &str, is_json: bool) -> Outcome {
        let scenario = scenario::find(scenario_name)?;
        let assembled = block_on(self.assemble(scenario))?;
        self.warn_if_pins_exceed_host(&assembled.plan);
        if is_json {
            println!("{}", assembled.plan.to_json());
        } else {
            for process in &assembled.plan.processes {
                println!(
                    "{:<18} {} ports {:?}",
                    process.name,
                    process.binary,
                    process.ports()
                );
            }
            for skipped in &assembled.skipped {
                println!("skipped {}: {}", skipped.name, skipped.reason);
            }
        }
        Ok(())
    }

    fn warn_if_pins_exceed_host(&self, plan: &Plan) {
        let Some(highest) = plan.max_core_id() else {
            return;
        };
        let cpus = std::thread::available_parallelism().map_or(0, |count| count.get());
        if cpus > 0 && highest as usize >= cpus {
            eprintln!(
                "warning: pinned runtime requests core {highest} but host reports {cpus} CPUs"
            );
        }
    }

    /// Starts the processes of `scenario_name` (only `only` when given),
    /// skipping the ones already running, and waits up to `wait` for each to
    /// report ready. Killing the CLI or its parent before the wait ends stops
    /// what this call started.
    pub fn up(
        &self,
        scenario_name: &str,
        only: &[String],
        build: bool,
        wait: Duration,
        is_json: bool,
    ) -> Outcome {
        procs::die_with_parent()?;
        let report = block_on(self.do_up(scenario_name, only, build, wait))?;
        print_report(&report, is_json);
        Ok(())
    }

    async fn do_up(
        &self,
        scenario_name: &str,
        only: &[String],
        build: bool,
        wait: Duration,
    ) -> Result<Report, Failure> {
        let scenario = scenario::find(scenario_name)?;
        let assembled = self.assemble(scenario).await?;
        self.warn_if_pins_exceed_host(&assembled.plan);
        self.start_plan(&assembled.plan, assembled.skipped, only, build, wait)
            .await
    }

    async fn start_plan(
        &self,
        plan: &Plan,
        left_out: Vec<Skipped>,
        only: &[String],
        build: bool,
        wait: Duration,
    ) -> Result<Report, Failure> {
        let mut prepared = self.prepare(plan, only, build)?;
        prepared.report.skipped.extend(
            left_out
                .into_iter()
                .filter(|skipped| only.is_empty() || only.contains(&skipped.name)),
        );
        self.save_state_for(plan, only)?;
        let names: Vec<String> = prepared
            .spawn
            .iter()
            .map(|process| process.name.clone())
            .collect();
        let mut report = prepared.report;
        let launched = tokio::select! {
            launched = self.launch(&prepared.spawn, wait) => launched,
            _ = shutdown_signal() => {
                self.rollback(&names)?;
                return Err(Failure::runtime(format!(
                    "interrupted: stopped {}",
                    if names.is_empty() { "nothing".to_string() } else { names.join(", ") }
                )));
            }
        }?;
        report.started = names;
        report.waited_ms = Some(launched);
        Ok(report)
    }

    fn prepare(&self, plan: &Plan, only: &[String], build: bool) -> Result<Prepared, Failure> {
        let store = self.store();
        let targets = select_processes(plan, only)?;
        let records = store.list()?;
        let mut report = Report::default();
        if only.is_empty() {
            let outside: Vec<&str> = records
                .iter()
                .filter(|record| record.is_running() && plan.process(&record.name).is_none())
                .map(|record| record.name.as_str())
                .collect();
            if !outside.is_empty() {
                return Err(Failure::runtime(format!(
                    "{} running outside scenario {}: run `rsx-cli cluster down` first",
                    outside.join(", "),
                    plan.scenario
                )));
            }
        }
        let mut spawn = Vec::new();
        for process in targets {
            let running = records
                .iter()
                .any(|record| record.name == process.name && record.is_running());
            if running {
                report
                    .skipped
                    .push(Skipped::new(&process.name, "already running"));
            } else {
                spawn.push(process.clone());
            }
        }
        if build {
            self.build_binaries(&spawn)?;
        }
        let refs: Vec<&Process> = spawn.iter().collect();
        let missing = procs::missing_binaries(&self.root, &refs);
        if !missing.is_empty() {
            return Err(Failure::runtime(format!(
                "binary not found: {} (build with `cargo build`, `make maker`, or pass --build)",
                missing.join(", ")
            )));
        }
        let is_cold = only.is_empty() && !records.iter().any(Record::is_running);
        let held = if is_cold {
            ports_held(&spawn)
        } else {
            Vec::new()
        };
        if !held.is_empty() {
            return Err(Failure::runtime(format!(
                "ports already in use: {} (another process holds them; `rsx-cli cluster ps` \
                 lists what the cluster started)",
                held.iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if spawn.iter().any(|process| !is_sidecar(&process.name)) {
            ensure_certs(&self.root)?;
        }
        for process in &spawn {
            self.make_directories(process)?;
        }
        Ok(Prepared { spawn, report })
    }

    fn make_directories(&self, process: &Process) -> Result<(), Failure> {
        for (key, value) in &process.env {
            let directory = if key.ends_with("_WAL_DIR") || key.ends_with("_ARCHIVE_DIR") {
                self.root.join(value)
            } else if key.ends_with("_TIP_FILE") {
                match self.root.join(value).parent() {
                    Some(parent) => parent.to_path_buf(),
                    None => continue,
                }
            } else {
                continue;
            };
            fs::create_dir_all(&directory).map_err(|error| {
                Failure::runtime(format!("failed to create {}: {error}", directory.display()))
            })?;
        }
        Ok(())
    }

    fn build_binaries(&self, spawn: &[Process]) -> Result<(), Failure> {
        let packages: BTreeSet<&str> = spawn
            .iter()
            .filter(|process| process.binary.starts_with("./target/"))
            .filter_map(|process| process.binary.rsplit('/').next())
            .collect();
        if packages.is_empty() {
            return Ok(());
        }
        let cargo = std::env::var("RSX_CARGO_BIN")
            .ok()
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "cargo".to_string());
        let mut command = Command::new(cargo);
        command.arg("build").current_dir(&self.root);
        for package in packages {
            command.args(["-p", package]);
        }
        if self.release {
            command.arg("--release");
        }
        let status = command
            .stdout(Stdio::from(std::io::stderr()))
            .status()
            .map_err(|error| Failure::runtime(format!("failed to run cargo build: {error}")))?;
        if !status.success() {
            return Err(Failure::runtime(format!("cargo build failed: {status}")));
        }
        Ok(())
    }

    fn save_state_for(&self, plan: &Plan, only: &[String]) -> Result<(), Failure> {
        let store = self.store();
        if !only.is_empty() && store.load_state()?.is_some() {
            return Ok(());
        }
        store.save_state(&State {
            scenario: plan.scenario.clone(),
            release: self.release,
            pin_cores: self.pin_cores,
            processes: plan
                .processes
                .iter()
                .map(|process| process.name.clone())
                .collect(),
        })
    }

    /// Spawns the daemons, waits for them, then spawns the sidecars (they
    /// need a gateway and marketdata that answer). Returns the milliseconds
    /// waited.
    async fn launch(&self, spawn: &[Process], wait: Duration) -> Result<u64, Failure> {
        let started_at = Instant::now();
        let store = self.store();
        let daemons: Vec<&Process> = spawn.iter().filter(|p| !is_sidecar(&p.name)).collect();
        let sidecars: Vec<&Process> = spawn.iter().filter(|p| is_sidecar(&p.name)).collect();
        let mut daemon_records = Vec::new();
        for process in daemons {
            daemon_records.push((process, self.spawn_one(&store, process)?));
            sleep(SPAWN_GAP).await;
        }
        self.wait_ready(&daemon_records, wait).await?;
        let mut sidecar_records = Vec::new();
        for process in sidecars {
            sidecar_records.push((process, self.spawn_one(&store, process)?));
        }
        if !sidecar_records.is_empty() {
            sleep(SIDECAR_GRACE).await;
            for (process, record) in &sidecar_records {
                if !record.is_running() {
                    return Err(self.exited_error(process, record));
                }
            }
        }
        Ok(started_at.elapsed().as_millis() as u64)
    }

    fn spawn_one(&self, store: &Store, process: &Process) -> Result<Record, Failure> {
        let (_, record) = procs::spawn(&self.root, process)?;
        store.save(&record)?;
        Ok(record)
    }

    async fn wait_ready(
        &self,
        started: &[(&Process, Record)],
        wait: Duration,
    ) -> Result<(), Failure> {
        let deadline = Instant::now() + wait;
        let mut pending: Vec<&(&Process, Record)> = started.iter().collect();
        loop {
            let mut waiting = Vec::new();
            for entry in pending {
                let (process, record) = entry;
                if !record.is_running() {
                    return Err(self.exited_error(process, record));
                }
                let is_up = match process.health_addr() {
                    Some(address) => probe::is_up(address, probe_for(process)).await,
                    None => true,
                };
                if !is_up {
                    waiting.push(entry);
                }
            }
            pending = waiting;
            if pending.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(not_ready_error(&pending, wait).await);
            }
            sleep(READY_POLL).await;
        }
    }

    fn exited_error(&self, process: &Process, record: &Record) -> Failure {
        let tail = log_tail(&self.root.join(&record.log));
        Failure::runtime(format!(
            "{} exited during startup (pid {}); {} ends:\n{tail}",
            process.name, record.pid, record.log
        ))
    }

    fn rollback(&self, names: &[String]) -> Result<(), Failure> {
        let store = self.store();
        let mut records = Vec::new();
        for name in names.iter().rev() {
            if let Some(record) = store.load(name)? {
                records.push(record);
            }
        }
        procs::stop(&records, STOP_GRACE);
        for record in &records {
            store.remove(&record.name)?;
        }
        Ok(())
    }

    /// Stops one process and starts it again from the plan of the last `up`
    /// (or of `scenario_name`), waiting up to `wait` for it to report ready.
    pub fn restart(
        &self,
        name: &str,
        scenario_name: Option<&str>,
        wait: Duration,
        is_json: bool,
    ) -> Outcome {
        procs::die_with_parent()?;
        let report = block_on(self.do_restart(name, scenario_name, wait))?;
        print_report(&report, is_json);
        Ok(())
    }

    async fn do_restart(
        &self,
        name: &str,
        scenario_name: Option<&str>,
        wait: Duration,
    ) -> Result<Report, Failure> {
        let store = self.store();
        let state = store.load_state()?;
        let scenario_name = match (scenario_name, &state) {
            (Some(scenario_name), _) => scenario_name.to_string(),
            (None, Some(state)) => state.scenario.clone(),
            (None, None) => {
                return Err(Failure::misuse(
                    "no cluster has been started: run `rsx-cli cluster up` or pass --scenario",
                ))
            }
        };
        let scenario = scenario::find(&scenario_name)?;
        let recorded = state.as_ref();
        let cluster = Cluster {
            release: self.release || recorded.is_some_and(|state| state.release),
            pin_cores: self.pin_cores || recorded.is_some_and(|state| state.pin_cores),
            ..self.clone()
        };
        let assembled = cluster.assemble(scenario).await?;
        cluster
            .restart_plan(&assembled.plan, &assembled.skipped, name, wait)
            .await
    }

    async fn restart_plan(
        &self,
        plan: &Plan,
        left_out: &[Skipped],
        name: &str,
        wait: Duration,
    ) -> Result<Report, Failure> {
        if plan.process(name).is_none() {
            let reason = left_out
                .iter()
                .find(|skipped| skipped.name == name)
                .map(|skipped| format!(" ({})", skipped.reason))
                .unwrap_or_default();
            return Err(Failure::misuse(format!(
                "{name:?} is not in scenario {}{reason}",
                plan.scenario
            )));
        }
        let store = self.store();
        let mut stopped = Vec::new();
        if let Some(record) = store.load(name)? {
            let results = procs::stop(std::slice::from_ref(&record), STOP_GRACE);
            if results.first() != Some(&Stopped::NotRunning) {
                stopped.push(name.to_string());
            }
            store.remove(name)?;
        }
        let mut report = self
            .start_plan(plan, Vec::new(), &[name.to_string()], false, wait)
            .await?;
        report.stopped = stopped;
        Ok(report)
    }
}

/// The ports `spawn` names that something already holds. Asked of a cold
/// start only: with part of the cluster running, a port is legitimately held
/// by a sibling, and a daemon that cannot bind fails its own start.
fn ports_held(spawn: &[Process]) -> Vec<u16> {
    let wanted: Vec<u16> = spawn
        .iter()
        .flat_map(Process::ports)
        .collect::<BTreeSet<u16>>()
        .into_iter()
        .collect();
    procs::ports_in_use(&wanted)
}

pub(crate) fn print_report(report: &Report, is_json: bool) {
    if is_json {
        println!("{}", report.to_json());
    } else {
        let text = report.to_text();
        if !text.is_empty() {
            println!("{text}");
        }
    }
}

async fn not_ready_error(pending: &[&(&Process, Record)], wait: Duration) -> Failure {
    let mut described = Vec::new();
    for entry in pending {
        let process = entry.0;
        let state = match process.health_addr() {
            Some(address) => probe::state(address).await,
            None => None,
        };
        described.push(format!(
            "{} ({})",
            process.name,
            state.unwrap_or_else(|| "no health answer".to_string())
        ));
    }
    Failure::runtime(format!(
        "not ready after {} s: {}; the processes are left running, see log/<name>.log, \
         `rsx-cli cluster down` stops them",
        wait.as_secs(),
        described.join(", ")
    ))
}

fn log_tail(path: &std::path::Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return format!("(cannot read {})", path.display());
    };
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(LOG_TAIL_LINES);
    lines
        .iter()
        .skip(skip)
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Provisions the replication certs under `<root>/certs` when absent, or when
/// `cert.pem` is a copy of `ca.pem` (a CA used as the server leaf, which
/// rustls rejects).
fn ensure_certs(root: &std::path::Path) -> Result<(), Failure> {
    let cert = root.join("certs/cert.pem");
    let ca = root.join("certs/ca.pem");
    let mut force = false;
    if cert.exists() {
        if !ca.exists() {
            return Ok(());
        }
        match (fs::read(&cert), fs::read(&ca)) {
            (Ok(cert_bytes), Ok(ca_bytes)) if cert_bytes == ca_bytes => force = true,
            _ => return Ok(()),
        }
    }
    let mut command = Command::new("sh");
    command
        .arg("scripts/gen-snakeoil-certs.sh")
        .current_dir(root);
    if force {
        command.arg("--force");
    }
    let output = command.output().map_err(|error| {
        Failure::runtime(format!("failed to run gen-snakeoil-certs.sh: {error}"))
    })?;
    if !output.status.success() {
        return Err(Failure::runtime(format!(
            "failed to generate replication certs: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Prints the known scenarios, with the load a stress scenario drives.
pub fn scenarios(is_json: bool) -> Outcome {
    if is_json {
        let rows: Vec<Value> = scenario::SCENARIOS
            .iter()
            .map(|scenario| {
                json!({
                    "name": scenario.name,
                    "symbols": scenario.symbols,
                    "replication": scenario.replication.label(),
                    "gateways": scenario.gateways,
                    "load": scenario.load.map(|load| json!({
                        "rate": load.rate,
                        "duration": load.duration_s,
                        "type": load.kind,
                    })),
                })
            })
            .collect();
        println!("{}", json!({ "scenarios": rows }));
    } else {
        for scenario in &scenario::SCENARIOS {
            println!(
                "{:<14} {} symbols, {} gateways, replication {}",
                scenario.name,
                scenario.symbols,
                scenario.gateways,
                scenario.replication.label()
            );
        }
    }
    Ok(())
}

/// Stops the processes the cluster started under `root` (only `only` when
/// given), in reverse start order. Nothing else is signalled.
pub fn down(root: &Path, only: &[String], is_json: bool) -> Outcome {
    let report = do_down(root, only)?;
    print_report(&report, is_json);
    Ok(())
}

pub(crate) fn do_down(root: &Path, only: &[String]) -> Result<Report, Failure> {
    let store = Store::new(root);
    let mut report = Report::default();
    let mut records = store.list()?;
    if let Some(state) = store.load_state()? {
        records.sort_by_key(|record| {
            std::cmp::Reverse(state.processes.iter().position(|name| *name == record.name))
        });
    }
    for name in only {
        if !records.iter().any(|record| record.name == *name) {
            report
                .skipped
                .push(Skipped::new(name, "no record: not started by the cluster"));
        }
    }
    records.retain(|record| only.is_empty() || only.contains(&record.name));
    let results = procs::stop(&records, STOP_GRACE);
    for (record, result) in records.iter().zip(results) {
        match result {
            Stopped::NotRunning => report
                .skipped
                .push(Skipped::new(&record.name, "already exited; record removed")),
            Stopped::Terminated | Stopped::Killed => report.stopped.push(record.name.clone()),
        }
        store.remove(&record.name)?;
    }
    if store.list()?.is_empty() {
        store.remove_state()?;
    }
    Ok(report)
}

/// One row per process of the last `up` and per record under `root`: running,
/// exited (a record whose process is gone) or absent (planned, never started).
/// `--json` also names the scenario of the last `up`, `null` before any.
pub fn ps(root: &Path, is_json: bool) -> Outcome {
    let statuses = statuses(root)?;
    if is_json {
        let rows: Vec<Value> = statuses.iter().map(Status::to_json).collect();
        let scenario = Store::new(root).load_state()?.map(|state| state.scenario);
        println!("{}", json!({ "scenario": scenario, "processes": rows }));
    } else {
        for status in &statuses {
            println!(
                "{:<18} {:<8} {:>8} {}",
                status.name,
                status.state.label(),
                status.pid.map_or("-".to_string(), |pid| pid.to_string()),
                status.log
            );
        }
    }
    Ok(())
}

/// The rows `ps` prints.
pub fn statuses(root: &Path) -> Result<Vec<Status>, Failure> {
    let store = Store::new(root);
    let records = store.list()?;
    let mut names: Vec<String> = store
        .load_state()?
        .map(|state| state.processes)
        .unwrap_or_default();
    for record in &records {
        if !names.contains(&record.name) {
            names.push(record.name.clone());
        }
    }
    Ok(names
        .into_iter()
        .map(
            |name| match records.iter().find(|record| record.name == name) {
                Some(record) => Status {
                    pid: Some(record.pid),
                    state: if record.is_running() {
                        ProcessState::Running
                    } else {
                        ProcessState::Exited
                    },
                    started_ms: Some(record.started_ms),
                    log: record.log.clone(),
                    name,
                },
                None => Status {
                    pid: None,
                    state: ProcessState::Absent,
                    started_ms: None,
                    log: format!("log/{name}.log"),
                    name,
                },
            },
        )
        .collect())
}
