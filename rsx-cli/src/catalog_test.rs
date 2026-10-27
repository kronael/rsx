use crate::catalog::redact;

#[test]
fn redact_drops_the_password_of_a_url() {
    assert_eq!(
        redact("postgres://rsx:hunter2@127.0.0.1:5432/rsx"),
        "postgres://rsx@127.0.0.1:5432/rsx"
    );
    assert_eq!(
        redact("postgres://rsx@127.0.0.1:5432/rsx"),
        "postgres://rsx@127.0.0.1:5432/rsx"
    );
}

#[test]
fn redact_drops_the_password_of_a_key_value_string() {
    assert_eq!(
        redact("host=127.0.0.1 user=postgres password=hunter2 dbname=rsx"),
        "host=127.0.0.1 user=postgres password=*** dbname=rsx"
    );
}
