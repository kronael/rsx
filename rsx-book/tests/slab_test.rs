use rsx_book::slab::Slab;
use rsx_book::slab::SlabItem;
use rsx_types::NONE;

#[derive(Default)]
struct TestItem {
    next: u32,
    value: u32,
}

impl SlabItem for TestItem {
    fn next(&self) -> u32 {
        self.next
    }
    fn set_next(&mut self, next: u32) {
        self.next = next;
    }
}

#[test]
fn slab_alloc_returns_sequential_indices() {
    let mut slab: Slab<TestItem> = Slab::new(10);
    assert_eq!(slab.alloc(), 0);
    assert_eq!(slab.alloc(), 1);
    assert_eq!(slab.alloc(), 2);
}

#[test]
fn slab_free_then_alloc_reuses_slot() {
    let mut slab: Slab<TestItem> = Slab::new(10);
    let a = slab.alloc();
    let _b = slab.alloc();
    slab.free(a);
    let c = slab.alloc();
    assert_eq!(c, a);
}

#[test]
fn slab_free_list_lifo_order() {
    let mut slab: Slab<TestItem> = Slab::new(10);
    let a = slab.alloc();
    let b = slab.alloc();
    let c = slab.alloc();
    slab.free(a);
    slab.free(b);
    slab.free(c);
    // LIFO: c, b, a
    assert_eq!(slab.alloc(), c);
    assert_eq!(slab.alloc(), b);
    assert_eq!(slab.alloc(), a);
}

#[test]
fn slab_alloc_exhausts_free_list_then_bumps() {
    let mut slab: Slab<TestItem> = Slab::new(10);
    let a = slab.alloc(); // 0
    slab.free(a);
    assert_eq!(slab.alloc(), 0); // reuse
    assert_eq!(slab.alloc(), 1); // bump
}

#[test]
fn slab_free_all_then_realloc_all() {
    let mut slab: Slab<TestItem> = Slab::new(5);
    let indices: Vec<u32> =
        (0..5).map(|_| slab.alloc()).collect();
    for &idx in &indices {
        slab.free(idx);
    }
    let mut realloc: Vec<u32> =
        (0..5).map(|_| slab.alloc()).collect();
    realloc.sort();
    assert_eq!(realloc, vec![0, 1, 2, 3, 4]);
}

#[test]
#[should_panic(expected = "slab exhausted")]
fn slab_capacity_limit() {
    let mut slab: Slab<TestItem> = Slab::new(3);
    slab.alloc();
    slab.alloc();
    slab.alloc();
    slab.alloc(); // should panic
}

#[test]
fn slab_len_and_capacity() {
    let mut slab: Slab<TestItem> = Slab::new(10);
    assert_eq!(slab.capacity(), 10);
    assert_eq!(slab.len(), 0);
    slab.alloc();
    slab.alloc();
    assert_eq!(slab.len(), 2);
}

#[test]
fn slab_get_and_get_mut() {
    let mut slab: Slab<TestItem> = Slab::new(10);
    let idx = slab.alloc();
    slab.get_mut(idx).value = 42;
    assert_eq!(slab.get(idx).value, 42);
}
