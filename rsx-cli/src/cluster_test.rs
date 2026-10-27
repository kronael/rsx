use crate::cluster::do_down;
use crate::cluster::statuses;
use crate::cluster::Cluster;
use crate::cluster::ProcessState;
use crate::cluster::Report;
use crate::cluster::Skipped;
use crate::health_fixture_utils::free_port;
use crate::health_fixture_utils::serve_later;
use crate::plan::Plan;
use crate::plan::Process;
use crate::procs::Record;
use crate::procs::Store;
use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::time::Duration;
use tempfile::tempdir;
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(10);

struct Fixture {
    root: TempDir,
    cluster: Cluster,
}

fn fixture() -> Fixture {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("certs")).unwrap();
    fs::write(root.path().join("certs/cert.pem"), "leaf").unwrap();
    fs::write(root.path().join("certs/ca.pem"), "authority").unwrap();
    let cluster = Cluster {
        root: root.path().to_path_buf(),
        database_url: "postgres://unused".to_string(),
        jwt_secret: "secret".to_string(),
        release: false,
        pin_cores: false,
        no_maker: false,
    };
    Fixture { root, cluster }
}

fn process(name: &str, script: &str, health_port: Option<u16>) -> Process {
    let mut env = BTreeMap::new();
    if let Some(port) = health_port {
        env.insert(
            "RSX_FAKE_HEALTH_ADDR".to_string(),
            format!("127.0.0.1:{port}"),
        );
    }
    Process {
        name: name.to_string(),
        binary: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        cwd: ".".to_string(),
        env,
    }
}

fn sleeper(name: &str, health_port: Option<u16>) -> Process {
    process(name, "exec sleep 60", health_port)
}

fn plan_of(processes: Vec<Process>) -> Plan {
    Plan {
        scenario: "fake".to_string(),
        processes,
    }
}

fn records(root: &Path) -> Vec<Record> {
    Store::new(root).list().unwrap()
}

fn pid_of(root: &Path, name: &str) -> u32 {
    Store::new(root).load(name).unwrap().unwrap().pid
}

fn bystander() -> Child {
    Command::new("sleep").arg("60").spawn().unwrap()
}

fn stop_bystander(mut child: Child) {
    child.kill().unwrap();
    child.wait().unwrap();
}

#[tokio::test]
async fn up_waits_for_ready_then_down_stops_only_what_it_started() {
    let fixture = fixture();
    let root = fixture.root.path();
    let other = bystander();
    let port = free_port();
    tokio::spawn(serve_later(
        port,
        Duration::from_millis(300),
        Duration::from_millis(900),
        "live",
    ));
    let plan = plan_of(vec![
        sleeper("fake-a", Some(port)),
        sleeper("fake-b", None),
        sleeper("maker", None),
    ]);

    let report = fixture
        .cluster
        .start_plan(
            &plan,
            vec![Skipped::new("auth", "not configured")],
            &[],
            false,
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(report.started, ["fake-a", "fake-b", "maker"]);
    assert_eq!(report.skipped, [Skipped::new("auth", "not configured")]);
    assert!(report.waited_ms.unwrap() >= 900, "{:?}", report.waited_ms);
    let rows = statuses(root).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.state == ProcessState::Running));
    let pids: Vec<u32> = records(root).iter().map(|record| record.pid).collect();

    let again = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap();
    assert!(again.started.is_empty());
    assert_eq!(
        again.skipped,
        [
            Skipped::new("fake-a", "already running"),
            Skipped::new("fake-b", "already running"),
            Skipped::new("maker", "already running"),
        ]
    );
    assert_eq!(
        records(root)
            .iter()
            .map(|record| record.pid)
            .collect::<Vec<_>>(),
        pids
    );

    let down: Report = do_down(root, &[]).unwrap();
    assert_eq!(down.stopped, ["maker", "fake-b", "fake-a"]);
    assert!(down.skipped.is_empty());
    assert!(records(root).is_empty());
    assert_eq!(Store::new(root).load_state().unwrap(), None);
    let mut other = other;
    assert!(
        other.try_wait().unwrap().is_none(),
        "bystander must survive down"
    );
    stop_bystander(other);
}

#[tokio::test]
async fn up_fails_naming_a_process_that_exits_during_startup() {
    let fixture = fixture();
    let port = free_port();
    tokio::spawn(serve_later(
        port,
        Duration::from_millis(100),
        Duration::ZERO,
        "live",
    ));
    let plan = plan_of(vec![
        sleeper("fake-a", Some(port)),
        process("crasher", "echo boom >&2; exit 3", Some(free_port())),
    ]);
    let failure = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap_err();
    assert!(
        failure.message().contains("crasher exited during startup"),
        "{}",
        failure.message()
    );
    assert!(failure.message().contains("boom"));
    do_down(fixture.root.path(), &[]).unwrap();
}

#[tokio::test]
async fn up_times_out_reporting_the_daemon_state_and_leaves_it_running() {
    let fixture = fixture();
    let root = fixture.root.path();
    let port = free_port();
    tokio::spawn(serve_later(
        port,
        Duration::from_millis(100),
        Duration::from_secs(3600),
        "warm_catchup",
    ));
    let plan = plan_of(vec![sleeper("risk-0", Some(port))]);
    let failure = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, Duration::from_millis(1200))
        .await
        .unwrap_err();
    assert!(
        failure.message().contains("not ready after 1 s"),
        "{}",
        failure.message()
    );
    assert!(failure.message().contains("risk-0 (warm_catchup)"));
    assert!(records(root)[0].is_running());
    do_down(root, &[]).unwrap();
}

#[tokio::test]
async fn a_warm_replica_counts_as_up_once_it_is_live() {
    let fixture = fixture();
    let port = free_port();
    tokio::spawn(serve_later(
        port,
        Duration::from_millis(100),
        Duration::ZERO,
        "live",
    ));
    let plan = plan_of(vec![sleeper("risk-0-replica-0", Some(port))]);
    let report = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap();
    assert_eq!(report.started, ["risk-0-replica-0"]);
    do_down(fixture.root.path(), &[]).unwrap();
}

#[tokio::test]
async fn a_sidecar_that_dies_at_once_fails_the_up() {
    let fixture = fixture();
    let plan = plan_of(vec![process("maker", "echo no gateway >&2; exit 1", None)]);
    let failure = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap_err();
    assert!(failure.message().contains("maker exited during startup"));
    assert!(failure.message().contains("no gateway"));
}

#[tokio::test]
async fn only_starts_and_down_only_stops_the_named_process() {
    let fixture = fixture();
    let root = fixture.root.path();
    let plan = plan_of(vec![sleeper("fake-a", None), sleeper("fake-b", None)]);
    let report = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &["fake-b".to_string()], false, WAIT)
        .await
        .unwrap();
    assert_eq!(report.started, ["fake-b"]);
    assert_eq!(records(root).len(), 1);
    fixture
        .cluster
        .start_plan(&plan, Vec::new(), &["fake-a".to_string()], false, WAIT)
        .await
        .unwrap();
    let down = do_down(root, &["fake-a".to_string(), "ghost".to_string()]).unwrap();
    assert_eq!(down.stopped, ["fake-a"]);
    assert_eq!(down.skipped.len(), 1);
    assert_eq!(down.skipped[0].name, "ghost");
    assert_eq!(records(root).len(), 1);
    assert!(Store::new(root).load_state().unwrap().is_some());
    do_down(root, &[]).unwrap();
    assert_eq!(Store::new(root).load_state().unwrap(), None);
}

#[tokio::test]
async fn an_unknown_only_name_is_misuse_listing_the_plan() {
    let fixture = fixture();
    let plan = plan_of(vec![sleeper("fake-a", None)]);
    let failure = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &["nope".to_string()], false, WAIT)
        .await
        .unwrap_err();
    assert_eq!(failure.exit_code(), 2);
    assert!(failure.message().contains("fake-a"));
}

#[tokio::test]
async fn restart_replaces_one_process_and_leaves_the_rest() {
    let fixture = fixture();
    let root = fixture.root.path();
    let plan = plan_of(vec![sleeper("fake-a", None), sleeper("fake-b", None)]);
    fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap();
    let old_a = pid_of(root, "fake-a");
    let old_b = pid_of(root, "fake-b");

    let report = fixture
        .cluster
        .restart_plan(&plan, &[], "fake-b", WAIT)
        .await
        .unwrap();
    assert_eq!(report.stopped, ["fake-b"]);
    assert_eq!(report.started, ["fake-b"]);
    assert_eq!(pid_of(root, "fake-a"), old_a);
    assert_ne!(pid_of(root, "fake-b"), old_b);
    assert!(records(root).iter().all(|record| record.is_running()));

    let failure = fixture
        .cluster
        .restart_plan(
            &plan,
            &[Skipped::new("auth", "not configured")],
            "auth",
            WAIT,
        )
        .await
        .unwrap_err();
    assert!(failure.message().contains("not configured"));
    do_down(root, &[]).unwrap();
}

#[tokio::test]
async fn restart_starts_a_process_that_crashed() {
    let fixture = fixture();
    let root = fixture.root.path();
    let plan = plan_of(vec![sleeper("fake-a", None)]);
    fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap();
    let record = records(root).remove(0);
    crate::procs::stop(std::slice::from_ref(&record), Duration::from_secs(5));
    assert_eq!(statuses(root).unwrap()[0].state, ProcessState::Exited);

    let report = fixture
        .cluster
        .restart_plan(&plan, &[], "fake-a", WAIT)
        .await
        .unwrap();
    assert!(report.stopped.is_empty());
    assert_eq!(report.started, ["fake-a"]);
    assert_eq!(statuses(root).unwrap()[0].state, ProcessState::Running);
    do_down(root, &[]).unwrap();
}

#[tokio::test]
async fn a_port_held_by_another_process_stops_the_up_before_any_spawn() {
    let fixture = fixture();
    let squatter = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    let plan = plan_of(vec![sleeper("fake-a", Some(port))]);
    let failure = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap_err();
    assert!(failure.message().contains("ports already in use"));
    assert!(failure.message().contains(&port.to_string()));
    assert!(records(fixture.root.path()).is_empty());
}

#[tokio::test]
async fn a_port_held_by_a_sibling_does_not_stop_a_partial_start() {
    let fixture = fixture();
    let root = fixture.root.path();
    let squatter = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    let mut peer = sleeper("fake-a", None);
    peer.env.insert(
        "RSX_FAKE_PEER_ADDR".to_string(),
        format!("127.0.0.1:{port}"),
    );
    let plan = plan_of(vec![peer, sleeper("fake-b", None)]);
    fixture
        .cluster
        .start_plan(&plan, Vec::new(), &["fake-b".to_string()], false, WAIT)
        .await
        .unwrap();

    let report = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &["fake-a".to_string()], false, WAIT)
        .await
        .unwrap();
    assert_eq!(report.started, ["fake-a"]);
    do_down(root, &[]).unwrap();
}

#[tokio::test]
async fn a_missing_binary_stops_the_up_before_any_spawn() {
    let fixture = fixture();
    let mut ghost = sleeper("ghost", None);
    ghost.binary = "./target/debug/ghost".to_string();
    let plan = plan_of(vec![sleeper("fake-a", None), ghost]);
    let failure = fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap_err();
    assert!(failure.message().contains("binary not found: ghost"));
    assert!(records(fixture.root.path()).is_empty());
}

#[tokio::test]
async fn a_running_process_outside_the_plan_stops_a_full_up() {
    let fixture = fixture();
    let root = fixture.root.path();
    let first = plan_of(vec![sleeper("fake-a", None)]);
    fixture
        .cluster
        .start_plan(&first, Vec::new(), &[], false, WAIT)
        .await
        .unwrap();
    let second = plan_of(vec![sleeper("fake-b", None)]);
    let failure = fixture
        .cluster
        .start_plan(&second, Vec::new(), &[], false, WAIT)
        .await
        .unwrap_err();
    assert!(failure
        .message()
        .contains("fake-a running outside scenario fake"));
    assert_eq!(records(root).len(), 1);
    do_down(root, &[]).unwrap();
}

#[tokio::test]
async fn rollback_stops_what_an_interrupted_up_started() {
    let fixture = fixture();
    let root = fixture.root.path();
    let plan = plan_of(vec![sleeper("fake-a", None), sleeper("fake-b", None)]);
    fixture
        .cluster
        .start_plan(&plan, Vec::new(), &[], false, WAIT)
        .await
        .unwrap();
    let started = records(root);
    fixture.cluster.rollback(&["fake-b".to_string()]).unwrap();
    assert_eq!(records(root).len(), 1);
    let gone = started
        .iter()
        .find(|record| record.name == "fake-b")
        .unwrap();
    assert!(!gone.is_running());
    do_down(root, &[]).unwrap();
}

#[test]
fn ps_lists_exited_and_absent_processes() {
    let fixture = fixture();
    let root = fixture.root.path();
    let store = Store::new(root);
    store
        .save_state(&crate::procs::State {
            scenario: "fake".to_string(),
            release: false,
            pin_cores: false,
            processes: vec!["fake-a".to_string(), "fake-b".to_string()],
        })
        .unwrap();
    store
        .save(&Record {
            name: "fake-a".to_string(),
            pid: 4_000_000,
            start_ticks: 1,
            started_ms: 99,
            log: "log/fake-a.log".to_string(),
        })
        .unwrap();
    let rows = statuses(root).unwrap();
    assert_eq!(rows[0].state, ProcessState::Exited);
    assert_eq!(rows[0].pid, Some(4_000_000));
    assert_eq!(rows[1].state, ProcessState::Absent);
    assert_eq!(rows[1].pid, None);
    assert_eq!(rows[1].log, "log/fake-b.log");
}

#[test]
fn report_json_carries_the_contract_keys() {
    let report = Report {
        started: vec!["gw-0".to_string()],
        stopped: Vec::new(),
        skipped: vec![Skipped::new("auth", "not configured")],
        waited_ms: Some(1500),
        cleared: Vec::new(),
    };
    let json = report.to_json();
    assert_eq!(json["started"][0], "gw-0");
    assert_eq!(json["skipped"][0]["name"], "auth");
    assert_eq!(json["skipped"][0]["reason"], "not configured");
    assert_eq!(json["waited_ms"], 1500);
    assert!(Report::default().to_json().get("waited_ms").is_none());
    assert!(Report::default().to_json().get("cleared").is_none());
}
