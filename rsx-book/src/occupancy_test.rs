use crate::occupancy::Occupancy;

/// Brute-force reference: lowest set bit >= from.
fn ref_next(bits: &[bool], from: u32) -> Option<u32> {
    (from as usize..bits.len())
        .find(|&i| bits[i])
        .map(|i| i as u32)
}

/// Brute-force reference: highest set bit <= from.
fn ref_prev(bits: &[bool], from: u32) -> Option<u32> {
    let hi = (from as usize).min(bits.len().saturating_sub(1));
    (0..=hi).rev().find(|&i| bits[i]).map(|i| i as u32)
}

#[test]
fn empty_finds_nothing() {
    let occ = Occupancy::new(1000);
    assert_eq!(occ.find_next(0), None);
    assert_eq!(occ.find_prev(999), None);
    assert_eq!(occ.find_first_in(0, 1000), None);
    assert_eq!(occ.find_last_in(0, 1000), None);
}

#[test]
fn single_bit_roundtrip() {
    let mut occ = Occupancy::new(1000);
    occ.set(500);
    assert_eq!(occ.find_next(0), Some(500));
    assert_eq!(occ.find_next(500), Some(500));
    assert_eq!(occ.find_next(501), None);
    assert_eq!(occ.find_prev(999), Some(500));
    assert_eq!(occ.find_prev(500), Some(500));
    assert_eq!(occ.find_prev(499), None);
    occ.clear(500);
    assert_eq!(occ.find_next(0), None);
}

#[test]
fn word_boundaries() {
    // Bits that stress the 64-bit word edges and summary propagation.
    let mut occ = Occupancy::new(10_000);
    for &b in &[0u32, 63, 64, 127, 128, 4095, 4096, 9999] {
        occ.set(b);
    }
    assert_eq!(occ.find_next(0), Some(0));
    assert_eq!(occ.find_next(1), Some(63));
    assert_eq!(occ.find_next(64), Some(64));
    assert_eq!(occ.find_next(65), Some(127));
    assert_eq!(occ.find_next(4097), Some(9999));
    assert_eq!(occ.find_next(10_000), None);
    assert_eq!(occ.find_prev(9998), Some(4096));
    assert_eq!(occ.find_prev(62), Some(0));
    assert_eq!(occ.find_last_in(64, 4096), Some(4095));
    assert_eq!(occ.find_last_in(64, 200), Some(128));
    assert_eq!(occ.find_first_in(65, 128), Some(127));
    assert_eq!(occ.find_first_in(129, 4095), None);
    assert_eq!(occ.find_first_in(129, 4096), Some(4095));
}

#[test]
fn range_queries_respect_bounds() {
    let mut occ = Occupancy::new(5000);
    occ.set(100);
    occ.set(200);
    occ.set(300);
    assert_eq!(occ.find_first_in(0, 100), None);
    assert_eq!(occ.find_first_in(0, 101), Some(100));
    assert_eq!(occ.find_first_in(101, 300), Some(200));
    assert_eq!(occ.find_last_in(0, 300), Some(200));
    assert_eq!(occ.find_last_in(201, 301), Some(300));
    assert_eq!(occ.find_last_in(201, 300), None);
}

#[test]
fn set_clear_keeps_summaries_consistent() {
    // Set every 7th bit, clear half, compare find_next/prev against a
    // brute-force reference at every position.
    let n = 3000u32;
    let mut occ = Occupancy::new(n);
    let mut bits = vec![false; n as usize];
    for i in (0..n).step_by(7) {
        occ.set(i);
        bits[i as usize] = true;
    }
    for i in (0..n).step_by(14) {
        occ.clear(i);
        bits[i as usize] = false;
    }
    for from in 0..n {
        assert_eq!(
            occ.find_next(from),
            ref_next(&bits, from),
            "find_next mismatch at {from}",
        );
        assert_eq!(
            occ.find_prev(from),
            ref_prev(&bits, from),
            "find_prev mismatch at {from}",
        );
    }
}

#[test]
fn randomized_against_reference() {
    // xorshift; deterministic. Random set/clear, compare every query.
    let n = 4096u32;
    let mut occ = Occupancy::new(n);
    let mut bits = vec![false; n as usize];
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut rng = || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..20_000 {
        let i = (rng() % n as u64) as u32;
        if rng() & 1 == 0 {
            occ.set(i);
            bits[i as usize] = true;
        } else {
            occ.clear(i);
            bits[i as usize] = false;
        }
    }
    for from in 0..n {
        assert_eq!(occ.find_next(from), ref_next(&bits, from));
        assert_eq!(occ.find_prev(from), ref_prev(&bits, from));
    }
}

#[test]
fn small_sizes_terminate() {
    for n in [0u32, 1, 2, 63, 64, 65] {
        let mut occ = Occupancy::new(n);
        if n > 0 {
            occ.set(n - 1);
            assert_eq!(occ.find_next(0), Some(n - 1));
            assert_eq!(occ.find_prev(n - 1), Some(n - 1));
        } else {
            assert_eq!(occ.find_next(0), None);
        }
    }
}
