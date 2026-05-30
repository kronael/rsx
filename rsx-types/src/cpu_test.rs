use crate::cpu::parse_cpu_list;

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
