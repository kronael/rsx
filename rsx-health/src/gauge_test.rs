use crate::gauge::Gauge;

#[test]
fn gauge_starts_at_zero() {
    assert_eq!(Gauge::default().get(), 0);
}

#[test]
fn gauge_set_replaces_and_add_accumulates() {
    let gauge = Gauge::default();
    gauge.set(5);
    gauge.add(3);
    gauge.add(2);
    assert_eq!(gauge.get(), 10);
    gauge.set(1);
    assert_eq!(gauge.get(), 1);
}

#[test]
fn gauge_add_wraps_at_u64_max() {
    let gauge = Gauge::default();
    gauge.set(u64::MAX);
    gauge.add(2);
    assert_eq!(gauge.get(), 1);
}
