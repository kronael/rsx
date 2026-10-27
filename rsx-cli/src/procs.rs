use crate::output::Failure;
use crate::plan::Process;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[cfg(test)]
#[path = "procs_test.rs"]
mod procs_test;

const POLL: Duration = Duration::from_millis(50);

/// A process the cluster started. `start_ticks` is the kernel start time from
/// `/proc/<pid>/stat`: a recycled pid has a different one, so a record never
/// points at a process the cluster did not start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub pid: u32,
    pub start_ticks: u64,
    pub started_ms: u64,
    pub log: String,
}

/// What the last `cluster up` set up: the names `ps` lists, absent ones
/// included, and what `restart` rebuilds from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub scenario: String,
    pub release: bool,
    pub pin_cores: bool,
    pub processes: Vec<String>,
}

/// How `stop` ended one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    NotRunning,
    Terminated,
    Killed,
}

/// The records and state under `<root>/tmp/cluster`.
pub struct Store {
    dir: PathBuf,
}

fn read_stat(pid: u32) -> Option<(char, u64)> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = text.rsplit_once(')')?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let start_ticks = fields.get(19)?.parse().ok()?;
    Some((state, start_ticks))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn send_signal(pid: u32, signal: libc::c_int) {
    // SAFETY: kill(2) takes two integers and touches no memory; a pid that
    // is gone answers ESRCH, which the callers treat as stopped.
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

/// Arranges for this process to get SIGTERM when its parent dies.
pub fn die_with_parent() -> Result<(), Failure> {
    // SAFETY: prctl(PR_SET_PDEATHSIG) takes integers and touches no memory.
    let status = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
    if status != 0 {
        return Err(Failure::runtime(
            "failed to arrange the parent-death signal",
        ));
    }
    Ok(())
}

impl Record {
    /// True while the process the record names is alive (a zombie is not).
    pub fn is_running(&self) -> bool {
        match read_stat(self.pid) {
            Some((state, start_ticks)) => {
                start_ticks == self.start_ticks && state != 'Z' && state != 'X'
            }
            None => false,
        }
    }
}

/// SIGTERM every running record in the order given, wait up to `grace` for
/// them all, then SIGKILL the survivors. Records that are not running (gone,
/// or a recycled pid) are never signalled. Results are in input order.
pub fn stop(records: &[Record], grace: Duration) -> Vec<Stopped> {
    let mut results: Vec<Stopped> = records
        .iter()
        .map(|record| {
            if record.is_running() {
                send_signal(record.pid, libc::SIGTERM);
                Stopped::Terminated
            } else {
                Stopped::NotRunning
            }
        })
        .collect();
    wait_until_gone(records, &results, grace);
    for (record, result) in records.iter().zip(results.iter_mut()) {
        if *result == Stopped::Terminated && record.is_running() {
            send_signal(record.pid, libc::SIGKILL);
            *result = Stopped::Killed;
        }
    }
    wait_until_gone(records, &results, Duration::from_secs(2));
    results
}

fn wait_until_gone(records: &[Record], results: &[Stopped], limit: Duration) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        let alive = records
            .iter()
            .zip(results)
            .any(|(record, result)| *result != Stopped::NotRunning && record.is_running());
        if !alive {
            return;
        }
        sleep(POLL);
    }
}

impl Store {
    pub fn new(root: &Path) -> Self {
        Self {
            dir: root.join("tmp").join("cluster"),
        }
    }

    fn records_dir(&self) -> PathBuf {
        self.dir.join("pids")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.records_dir().join(format!("{name}.json"))
    }

    pub fn save(&self, record: &Record) -> Result<(), Failure> {
        write_json(&self.record_path(&record.name), record)
    }

    pub fn load(&self, name: &str) -> Result<Option<Record>, Failure> {
        read_json(&self.record_path(name))
    }

    /// Every record, ordered by name.
    pub fn list(&self) -> Result<Vec<Record>, Failure> {
        let dir = self.records_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(Failure::runtime(format!(
                    "failed to read {}: {error}",
                    dir.display()
                )))
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    Failure::runtime(format!("failed to read {}: {error}", dir.display()))
                })?
                .path();
            if let Some(record) = read_json(&path)? {
                records.push(record);
            }
        }
        records.sort_by(|left: &Record, right| left.name.cmp(&right.name));
        Ok(records)
    }

    pub fn remove(&self, name: &str) -> Result<(), Failure> {
        remove_file(&self.record_path(name))
    }

    pub fn save_state(&self, state: &State) -> Result<(), Failure> {
        write_json(&self.state_path(), state)
    }

    pub fn load_state(&self) -> Result<Option<State>, Failure> {
        read_json(&self.state_path())
    }

    pub fn remove_state(&self) -> Result<(), Failure> {
        remove_file(&self.state_path())
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Failure::runtime(format!("failed to create {}: {error}", parent.display()))
        })?;
    }
    let text = serde_json::to_string(value).map_err(|error| {
        Failure::runtime(format!("failed to encode {}: {error}", path.display()))
    })?;
    fs::write(path, text)
        .map_err(|error| Failure::runtime(format!("failed to write {}: {error}", path.display())))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, Failure> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Failure::runtime(format!(
                "failed to read {}: {error}",
                path.display()
            )))
        }
    };
    serde_json::from_str(&text).map(Some).map_err(|error| {
        Failure::runtime(format!(
            "{} is not a cluster record ({error}); delete it",
            path.display()
        ))
    })
}

fn remove_file(path: &Path) -> Result<(), Failure> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Failure::runtime(format!(
            "failed to remove {}: {error}",
            path.display()
        ))),
    }
}

/// Names of the processes whose binary does not exist under `root`.
pub fn missing_binaries(root: &Path, processes: &[&Process]) -> Vec<String> {
    processes
        .iter()
        .filter(|process| !root.join(&process.binary).exists())
        .map(|process| format!("{} ({})", process.name, process.binary))
        .collect()
}

/// The ports among `ports` that something already listens on or has bound,
/// TCP or UDP.
pub fn ports_in_use(ports: &[u16]) -> Vec<u16> {
    ports
        .iter()
        .copied()
        .filter(|port| {
            let tcp = TcpListener::bind(("0.0.0.0", *port));
            let udp = UdpSocket::bind(("0.0.0.0", *port));
            is_in_use(&tcp) || is_in_use(&udp)
        })
        .collect()
}

fn is_in_use<T>(bound: &std::io::Result<T>) -> bool {
    matches!(bound, Err(error) if error.kind() == std::io::ErrorKind::AddrInUse)
}

/// Starts `process` detached in its own process group, with `<root>/log/<name>.log`
/// as stdout and stderr (appended). The caller's environment is inherited and
/// the process env added. Returns the child and its record.
pub fn spawn(root: &Path, process: &Process) -> Result<(Child, Record), Failure> {
    let binary = root.join(&process.binary);
    if !binary.exists() {
        return Err(Failure::runtime(format!(
            "{}: binary not found: {} (build it first)",
            process.name,
            binary.display()
        )));
    }
    let log_dir = root.join("log");
    fs::create_dir_all(&log_dir).map_err(|error| {
        Failure::runtime(format!("failed to create {}: {error}", log_dir.display()))
    })?;
    let log_path = log_dir.join(format!("{}.log", process.name));
    let log = open_log(&log_path)?;
    let log_for_stderr = log.try_clone().map_err(|error| {
        Failure::runtime(format!("failed to open {}: {error}", log_path.display()))
    })?;
    let child = Command::new(&binary)
        .args(&process.args)
        .envs(&process.env)
        .current_dir(root.join(&process.cwd))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_for_stderr))
        .process_group(0)
        .spawn()
        .map_err(|error| {
            Failure::runtime(format!(
                "{}: failed to start {}: {error}",
                process.name,
                binary.display()
            ))
        })?;
    let record = Record {
        name: process.name.clone(),
        pid: child.id(),
        start_ticks: read_stat(child.id()).map_or(0, |(_, start_ticks)| start_ticks),
        started_ms: now_ms(),
        log: format!("log/{}.log", process.name),
    };
    Ok((child, record))
}

fn open_log(path: &Path) -> Result<File, Failure> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| Failure::runtime(format!("failed to open {}: {error}", path.display())))
}
