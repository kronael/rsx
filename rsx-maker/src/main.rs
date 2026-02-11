use rsx_types::install_panic_handler;
use std::env;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let symbol_id: u32 = env::var("RSX_MAKER_SYMBOL_ID")
        .expect("RSX_MAKER_SYMBOL_ID required")
        .parse()
        .expect("invalid RSX_MAKER_SYMBOL_ID");

    let me_addr = env::var("RSX_ME_CMP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9100".into());

    info!(
        "rsx-maker stub: symbol_id={} me_addr={}",
        symbol_id, me_addr,
    );

    // TODO: connect to ME via CMP, post two-sided quotes
    // around mid price, react to fills
}
