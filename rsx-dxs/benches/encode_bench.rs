use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::*;

fn bench_fill_record_encode(c: &mut Criterion) {
    let record = FillRecord {
        preamble: PayloadPreamble {
            seq: 1,
            ver: 1,
            kind: 0,
            _pad0: 0,
            len: std::mem::size_of::<FillRecord>() as u32,
        },
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: 50000,
        qty: 100,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    c.bench_function("fill_record_encode", |b| {
        b.iter(|| encode_fill_record(1, &record));
    });
}

fn bench_fill_record_decode(c: &mut Criterion) {
    let record = FillRecord {
        preamble: PayloadPreamble {
            seq: 1,
            ver: 1,
            kind: 0,
            _pad0: 0,
            len: std::mem::size_of::<FillRecord>() as u32,
        },
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: 50000,
        qty: 100,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let encoded = encode_fill_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];

    c.bench_function("fill_record_decode", |b| {
        b.iter(|| decode_fill_record(payload));
    });
}

fn bench_crc32_compute_128b(c: &mut Criterion) {
    let data = [0u8; 128];
    c.bench_function("crc32_compute_128b", |b| {
        b.iter(|| compute_crc32(&data));
    });
}

fn bench_header_encode_decode(c: &mut Criterion) {
    let header = WalHeader::new(0, 64, 1, 0xDEADBEEF);
    c.bench_function("header_encode", |b| {
        b.iter(|| header.to_bytes());
    });

    let bytes = header.to_bytes();
    c.bench_function("header_decode", |b| {
        b.iter(|| WalHeader::from_bytes(&bytes));
    });
}

criterion_group!(
    benches,
    bench_fill_record_encode,
    bench_fill_record_decode,
    bench_crc32_compute_128b,
    bench_header_encode_decode
);
criterion_main!(benches);
