use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_gateway::config::load_gateway_config;
use rsx_types::install_panic_handler;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = load_gateway_config();

    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            .expect("invalid RSX_RISK_CMP_ADDR");
    let gw_addr: SocketAddr =
        env::var("RSX_GW_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".into())
            .parse()
            .expect("invalid RSX_GW_CMP_ADDR");
    let wal_dir = env::var("RSX_GW_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".into());

    // CMP/UDP: send orders to Risk
    let mut cmp_sender = CmpSender::new(
        risk_addr,
        0,
        &PathBuf::from(&wal_dir),
    )
    .expect("failed to create CMP sender");

    // CMP/UDP: receive responses from Risk
    let mut cmp_receiver = CmpReceiver::new(
        gw_addr, risk_addr, 0,
    )
    .expect("failed to bind CMP receiver");

    info!(
        "gateway started on {}",
        config.listen_addr,
    );

    // Run monoio event loop
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .build()
    .expect("failed to build monoio runtime");

    let listen_addr = config.listen_addr.clone();
    rt.block_on(async move {
        // Spawn WS accept loop
        let ws_addr = listen_addr;
        monoio::spawn(async move {
            if let Err(e) =
                rsx_gateway::ws::ws_accept_loop(
                    &ws_addr,
                    |_stream| {
                        // TODO: spawn per-connection
                        // handler with protocol parsing
                    },
                )
                .await
            {
                tracing::error!(
                    "ws accept error: {e}"
                );
            }
        });

        // CMP polling loop (yields to monoio)
        loop {
            while let Some((_hdr, _payload)) =
                cmp_receiver.try_recv()
            {
                // TODO: route to client connections
            }

            let _ = cmp_sender.tick();
            cmp_receiver.tick();
            cmp_sender.recv_control();

            // Yield to monoio scheduler
            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
        }
    });
}
