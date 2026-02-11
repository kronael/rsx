use rsx_types::install_panic_handler;
use std::env;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let gw_url = env::var("RSX_SIM_GW_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:8080".into());

    let num_users: u32 = env::var("RSX_SIM_USERS")
        .unwrap_or_else(|_| "10".into())
        .parse()
        .expect("invalid RSX_SIM_USERS");

    info!(
        "rsx-sim stub: gw_url={} users={}",
        gw_url, num_users,
    );

    // TODO: spawn N user tasks via tokio, connect WS,
    // send orders with profiles (directional, random,
    // aggressive), measure p50/p99/max latency
}
