use rsx_dxs::cmp::CmpReceiver;
use rsx_marketdata::config::load_marketdata_config;
use rsx_types::install_panic_handler;
use std::env;
use std::net::SocketAddr;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = load_marketdata_config();

    let mkt_addr: SocketAddr =
        env::var("RSX_MKT_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9103".into())
            .parse()
            .expect("invalid RSX_MKT_CMP_ADDR");
    let me_addr: SocketAddr =
        env::var("RSX_ME_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".into())
            .parse()
            .expect("invalid RSX_ME_CMP_ADDR");

    // CMP/UDP: receive events from ME
    let mut cmp_receiver = CmpReceiver::new(
        mkt_addr, me_addr, 0,
    )
    .expect("failed to bind marketdata CMP");

    info!(
        "marketdata started on {}",
        config.listen_addr,
    );

    // Run monoio event loop
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .build()
    .expect("failed to build monoio runtime");

    rt.block_on(async move {
        // TODO: spawn WS broadcast accept loop

        // CMP polling + shadow book update loop
        loop {
            while let Some((_hdr, _payload)) =
                cmp_receiver.try_recv()
            {
                // TODO: decode event, update shadow
                // book, broadcast to WS subscribers
            }

            cmp_receiver.tick();

            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
        }
    });
}
