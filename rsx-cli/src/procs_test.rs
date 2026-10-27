use crate::plan::Process;
use crate::procs::missing_binaries;
use crate::procs::ports_in_use;
use crate::procs::spawn;
use crate::procs::stop;
use crate::procs::Record;
use crate::procs::State;
use crate::procs::Stopped;
use crate::procs::Store;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::process::Command;
use std::time::Duration;
use tempfile::tempdir;

fn shell(name: &str, script: &str) -> Process {
    Process {
        name: name.to_string(),
        binary: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        cwd: ".".to_string(),
        env: BTreeMap::new(),
    }
}

fn record(name: &str, pid: u32) -> Record {
    Record {
        name: name.to_string(),
        pid,
        start_ticks: 7,
        started_ms: 1,
        log: format!("log/{name}.log"),
    }
}

#[test]
fn store_round_trips_records_and_state() {
    let root = tempdir().unwrap();
    let store = Store::new(root.path());
    assert!(store.list().unwrap().is_empty());
    assert_eq!(store.load("gw-0").unwrap(), None);
    store.save(&record("gw-0", 11)).unwrap();
    store.save(&record("risk-0", 12)).unwrap();
    assert_eq!(store.load("gw-0").unwrap(), Some(record("gw-0", 11)));
    let names: Vec<String> = store.list().unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["gw-0", "risk-0"]);
    store.remove("gw-0").unwrap();
    store.remove("gw-0").unwrap();
    assert_eq!(store.list().unwrap().len(), 1);

    assert_eq!(store.load_state().unwrap(), None);
    let state = State {
        scenario: "minimal".to_string(),
        release: false,
        pin_cores: false,
        processes: vec!["gw-0".to_string()],
    };
    store.save_state(&state).unwrap();
    assert_eq!(store.load_state().unwrap(), Some(state));
    store.remove_state().unwrap();
    assert_eq!(store.load_state().unwrap(), None);
}

#[test]
fn a_corrupt_record_is_an_error_naming_the_file() {
    let root = tempdir().unwrap();
    let store = Store::new(root.path());
    store.save(&record("gw-0", 11)).unwrap();
    let path = root.path().join("tmp/cluster/pids/gw-0.json");
    std::fs::write(&path, "not json").unwrap();
    let failure = store.load("gw-0").unwrap_err();
    assert!(failure.message().contains("gw-0.json"));
}

#[test]
fn spawn_runs_detached_with_output_in_the_log() {
    let root = tempdir().unwrap();
    let (mut child, record) =
        spawn(root.path(), &shell("echoer", "echo hello; echo oops >&2")).unwrap();
    child.wait().unwrap();
    let log = std::fs::read_to_string(root.path().join(&record.log)).unwrap();
    assert!(log.contains("hello"));
    assert!(log.contains("oops"));
    assert_eq!(record.log, "log/echoer.log");
}

#[test]
fn stop_ends_a_running_process_and_a_dead_one_is_not_running() {
    let root = tempdir().unwrap();
    let (mut child, record) = spawn(root.path(), &shell("sleeper", "exec sleep 60")).unwrap();
    assert!(record.is_running());
    let results = stop(std::slice::from_ref(&record), Duration::from_secs(5));
    assert_eq!(results, [Stopped::Terminated]);
    child.wait().unwrap();
    assert!(!record.is_running());
    assert_eq!(
        stop(&[record], Duration::from_secs(1)),
        [Stopped::NotRunning]
    );
}

#[test]
fn stop_kills_a_process_that_ignores_sigterm() {
    let root = tempdir().unwrap();
    let script = "trap '' TERM; while true; do sleep 1; done";
    let (mut child, record) = spawn(root.path(), &shell("stubborn", script)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let results = stop(std::slice::from_ref(&record), Duration::from_millis(500));
    assert_eq!(results, [Stopped::Killed]);
    child.wait().unwrap();
    assert!(!record.is_running());
}

#[test]
fn a_record_with_the_wrong_start_time_is_never_signalled() {
    let mut bystander = Command::new("sleep").arg("60").spawn().unwrap();
    let mut forged = record("recycled", bystander.id());
    forged.start_ticks = 1;
    assert!(!forged.is_running());
    assert_eq!(
        stop(&[forged], Duration::from_millis(200)),
        [Stopped::NotRunning]
    );
    assert!(bystander.try_wait().unwrap().is_none());
    bystander.kill().unwrap();
    bystander.wait().unwrap();
}

#[test]
fn a_missing_binary_is_named() {
    let root = tempdir().unwrap();
    let mut ghost = shell("ghost", "true");
    ghost.binary = "./target/debug/ghost".to_string();
    let present = shell("present", "true");
    assert_eq!(
        missing_binaries(root.path(), &[&ghost, &present]),
        ["ghost (./target/debug/ghost)"]
    );
    let failure = spawn(root.path(), &ghost).unwrap_err();
    assert!(failure.message().contains("binary not found"));
}

#[test]
fn ports_in_use_sees_tcp_and_udp_binds() {
    let tcp = TcpListener::bind("0.0.0.0:0").unwrap();
    let udp = UdpSocket::bind("0.0.0.0:0").unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    let udp_port = udp.local_addr().unwrap().port();
    let free = {
        let probe = TcpListener::bind("0.0.0.0:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let mut held = ports_in_use(&[tcp_port, udp_port, free]);
    held.sort_unstable();
    let mut expected = vec![tcp_port, udp_port];
    expected.sort_unstable();
    assert_eq!(held, expected);
}
