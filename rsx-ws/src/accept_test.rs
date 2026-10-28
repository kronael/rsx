use crate::accept::ws_accept_loop;
use monoio::net::TcpStream;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

// accepted connection -> handler called with the client's own address
#[test]
fn handler_receives_the_peer_address() {
    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();
    runtime.block_on(async {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let peers: Rc<RefCell<Vec<SocketAddr>>> = Rc::default();
        let seen = peers.clone();
        monoio::spawn(async move {
            ws_accept_loop(&addr.to_string(), move |_, peer| {
                seen.borrow_mut().push(peer);
            })
            .await
            .unwrap();
        });
        monoio::time::sleep(Duration::from_millis(50)).await;

        let client = TcpStream::connect(addr).await.unwrap();
        monoio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(*peers.borrow(), [client.local_addr().unwrap()]);
    });
}
