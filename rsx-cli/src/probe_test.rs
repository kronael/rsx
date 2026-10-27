use crate::health_fixture_utils::bind;
use crate::health_fixture_utils::free_port;
use crate::health_fixture_utils::serve;
use crate::probe::is_up;
use crate::probe::state;
use crate::probe::status;
use crate::probe::Probe;
use std::time::Duration;

#[tokio::test]
async fn ready_is_503_until_the_daemon_serves_then_200() {
    let (listener, address) = bind().await;
    tokio::spawn(serve(listener, Duration::from_millis(400), "warm_catchup"));
    assert_eq!(status(&address, "/ready").await, Some(503));
    assert!(!is_up(&address, Probe::Ready).await);
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(status(&address, "/ready").await, Some(200));
    assert!(is_up(&address, Probe::Ready).await);
    assert!(is_up(&address, Probe::Live).await);
}

#[tokio::test]
async fn state_reads_the_label_from_metrics() {
    let (listener, address) = bind().await;
    tokio::spawn(serve(listener, Duration::from_secs(60), "warm_catchup"));
    assert_eq!(state(&address).await.as_deref(), Some("warm_catchup"));
}

#[tokio::test]
async fn nothing_listening_is_not_up_and_has_no_state() {
    let address = format!("127.0.0.1:{}", free_port());
    assert_eq!(status(&address, "/ready").await, None);
    assert!(!is_up(&address, Probe::Ready).await);
    assert_eq!(state(&address).await, None);
}

#[test]
fn probe_paths() {
    assert_eq!(Probe::Ready.path(), "/ready");
    assert_eq!(Probe::Live.path(), "/health");
}
