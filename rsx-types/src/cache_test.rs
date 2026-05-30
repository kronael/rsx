use crate::cache::Padded;
use crate::cache::PAD;

#[test]
fn align_matches_pad() {
    // Locks the repr(align(128)) literal to the PAD const.
    assert_eq!(core::mem::align_of::<Padded<u8>>(), PAD);
}

#[test]
fn size_is_at_least_a_pad_span() {
    assert_eq!(core::mem::size_of::<Padded<u64>>(), PAD);
}

#[test]
fn deref_reads_inner() {
    let p = Padded::new(7u64);
    assert_eq!(*p, 7);
}

#[test]
fn deref_mut_writes_inner() {
    let mut p = Padded::new(0u64);
    *p = 42;
    assert_eq!(p.into_inner(), 42);
}

#[test]
fn neighbours_sit_on_separate_lines() {
    // Two adjacent Padded values must be >= PAD bytes apart, so a write to one
    // never invalidates the other's line.
    let pair = [Padded::new(0u64), Padded::new(0u64)];
    let a = &pair[0] as *const _ as usize;
    let b = &pair[1] as *const _ as usize;
    assert!(b - a >= PAD, "neighbours {} apart, want >= {}", b - a, PAD);
}
