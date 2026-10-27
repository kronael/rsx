use crate::database::check_url;
use crate::output::Failure;

#[test]
fn well_formed_urls_pass() {
    for url in [
        "postgres://rsx:rsx@127.0.0.1:5432/rsx",
        "host=127.0.0.1 port=5432 user=postgres dbname=rsx",
    ] {
        assert!(check_url(url).is_ok(), "{url}");
    }
}

#[test]
fn a_malformed_url_is_misuse_and_does_not_echo_the_password() {
    for url in [
        "postgres://rsx:hunter2@127.0.0.1:notaport/rsx",
        "host=127.0.0.1 password=hunter2 sslmode=sometimes",
        "port=hunter2",
    ] {
        let failure = check_url(url).expect_err(url);
        assert!(matches!(failure, Failure::Misuse(_)), "{failure:?}");
        assert!(
            !failure.message().contains("hunter2"),
            "{}",
            failure.message()
        );
    }
}
