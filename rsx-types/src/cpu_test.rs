use crate::cpu::parse_cpu_list;
use crate::cpu::pin_current;

#[test]
fn parse_empty_is_empty() {
    assert!(parse_cpu_list("").is_empty());
}

#[test]
fn parse_single() {
    assert_eq!(parse_cpu_list("5"), vec![5]);
}

#[test]
fn parse_range() {
    assert_eq!(parse_cpu_list("2-4"), vec![2, 3, 4]);
}

#[test]
fn parse_mixed_list() {
    assert_eq!(parse_cpu_list("2-3,5,7-9"), vec![2, 3, 5, 7, 8, 9]);
}

#[test]
fn parse_ignores_garbage_tokens() {
    assert_eq!(parse_cpu_list("1,x,3-z,4"), vec![1, 4]);
}

fn allowed_cpus() -> Vec<usize> {
    core_affinity::get_core_ids()
        .unwrap()
        .iter()
        .map(|id| id.id)
        .collect()
}

fn current_cpus() -> Vec<usize> {
    // SAFETY: cpu_set_t is plain data; sched_getaffinity fills it for this thread.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        let rc = libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set);
        assert_eq!(rc, 0);
        (0..libc::CPU_SETSIZE as usize)
            .filter(|cpu| libc::CPU_ISSET(*cpu, &set))
            .collect()
    }
}

#[test]
fn pin_cpu_number_in_the_allowed_set_pins() {
    let cpu = *allowed_cpus().last().unwrap();
    let pinned = std::thread::spawn(move || (pin_current(cpu), current_cpus()))
        .join()
        .unwrap();
    assert_eq!(pinned, (true, vec![cpu]));
}

#[test]
fn pin_cpu_number_outside_the_allowed_set_is_refused() {
    let cpu = allowed_cpus().into_iter().max().unwrap() + 1;
    let refused = std::thread::spawn(move || {
        let before = current_cpus();
        (pin_current(cpu), before == current_cpus())
    })
    .join()
    .unwrap();
    assert_eq!(refused, (false, true));
}

#[test]
fn pin_is_a_cpu_number_not_an_index_into_the_allowed_set() {
    let allowed = allowed_cpus();
    let cpu = *allowed.last().unwrap();
    let other = allowed[0];
    let (again, elsewhere) = std::thread::spawn(move || {
        assert!(pin_current(cpu));
        (pin_current(cpu), pin_current(other))
    })
    .join()
    .unwrap();
    assert!(again);
    assert_eq!(elsewhere, other == cpu);
}
