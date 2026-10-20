//! specs/2/21-orderbook.md:451 (and specs/2/41-testing-matching.md M23): on a
//! warmed book and WAL buffer, matching an order and publishing its events
//! allocates nothing. The dedup set and the order index are growable by
//! design (notes/zero-heap.md) and sit outside the measured span.

use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalWriter;
use rsx_matching::wal::publish_events;
use rsx_matching::wal::CastPeer;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;
use std::net::UdpSocket;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

struct CountingAllocator;

fn count_allocation() {
    if COUNTING.get() {
        ALLOCATIONS.set(ALLOCATIONS.get() + 1);
    }
}

// SAFETY: every call forwards unchanged to `System`; counting touches only
// const-initialised thread-locals, which never allocate.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const SYMBOL_ID: u32 = 1;

fn order(user_id: u32, order_id_lo: u64, side: Side, tif: TimeInForce, qty: i64) -> IncomingOrder {
    IncomingOrder {
        price: 50_000,
        qty,
        remaining_qty: qty,
        side,
        tif,
        user_id,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo,
    }
}

struct Tile {
    book: Orderbook,
    wal_writer: WalWriter,
    risk: CastPeer,
    mkt: CastPeer,
    _wal_dir: tempfile::TempDir,
    _sockets: [UdpSocket; 2],
}

impl Tile {
    fn new() -> Self {
        let config = SymbolConfig {
            symbol_id: SYMBOL_ID,
            price_decimals: 0,
            qty_decimals: 0,
            tick_size: 1,
            lot_size: 1,
        };
        let wal_dir = tempfile::tempdir().unwrap();
        let sockets = [
            UdpSocket::bind("127.0.0.1:0").unwrap(),
            UdpSocket::bind("127.0.0.1:0").unwrap(),
        ];
        let risk = CastPeer::new(
            "risk",
            CastSender::new(sockets[0].local_addr().unwrap(), SYMBOL_ID, wal_dir.path()).unwrap(),
        );
        let mkt = CastPeer::new(
            "marketdata",
            CastSender::new(sockets[1].local_addr().unwrap(), SYMBOL_ID, wal_dir.path()).unwrap(),
        );
        Self {
            book: Orderbook::new(config, 1024, 50_000),
            wal_writer: WalWriter::new(SYMBOL_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap(),
            risk,
            mkt,
            _wal_dir: wal_dir,
            _sockets: sockets,
        }
    }

    /// Match `incoming`, then publish its events; returns the allocations
    /// made by just those two calls.
    fn match_and_publish(&mut self, mut incoming: IncomingOrder) -> u64 {
        ALLOCATIONS.set(0);
        COUNTING.set(true);
        process_new_order(&mut self.book, &mut incoming);
        publish_events(
            &mut self.wal_writer,
            &mut self.risk,
            &mut self.mkt,
            &self.book,
            SYMBOL_ID,
            2,
            1,
            1,
            1,
            1,
        )
        .unwrap();
        COUNTING.set(false);
        let allocations = ALLOCATIONS.get();
        self.wal_writer.flush().unwrap();
        allocations
    }

    /// Rest, partly fill, rest a remainder, then sweep: every order shape
    /// that reaches the book, one cycle starting and ending on an empty book.
    fn cycle(&mut self) -> u64 {
        let mut allocations = 0;
        for incoming in [
            order(1, 1, Side::Sell, TimeInForce::GTC, 10),
            order(2, 2, Side::Buy, TimeInForce::IOC, 4),
            order(2, 3, Side::Buy, TimeInForce::GTC, 20),
            order(1, 4, Side::Sell, TimeInForce::IOC, 30),
        ] {
            allocations += self.match_and_publish(incoming);
        }
        allocations
    }
}

#[test]
fn matching_and_publishing_allocate_nothing_on_a_warmed_tile() {
    let mut tile = Tile::new();
    for _ in 0..16 {
        tile.cycle();
    }

    let seq_before = tile.wal_writer.last_seq();
    let mut allocations = 0;
    for _ in 0..100 {
        allocations += tile.cycle();
    }

    assert!(tile.wal_writer.last_seq() > seq_before + 100 * 4);
    assert_eq!(allocations, 0, "allocations across 100 warmed cycles");
}

#[test]
fn counter_sees_an_allocation_inside_the_span() {
    ALLOCATIONS.set(0);
    COUNTING.set(true);
    let boxed = std::hint::black_box(Box::new(1u64));
    COUNTING.set(false);
    drop(boxed);

    assert_eq!(ALLOCATIONS.get(), 1);
}
