use rsx_gateway::jwt::validate_jwt;
use serde_json::Value;
use std::process::Command;
use std::process::Output;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";

fn run_token(args: &[&str], secret_env: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rsx-cli"));
    command.arg("token").arg("mint").args(args);
    command.env_remove("RSX_GW_JWT_SECRET");
    if let Some(secret) = secret_env {
        command.env("RSX_GW_JWT_SECRET", secret);
    }
    command.output().expect("failed to run rsx-cli")
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!("{}: {}", err, String::from_utf8_lossy(&out.stdout));
    })
}

#[test]
fn test_minted_token_passes_gateway_validation() {
    let out = run_token(&["-u", "42", "--json"], Some(SECRET));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let value = stdout_json(&out);
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["exp", "token", "user_id"]);
    assert_eq!(value["user_id"], 42);
    assert_eq!(
        validate_jwt(value["token"].as_str().unwrap(), SECRET),
        Ok(42)
    );
}

#[test]
fn test_secret_flag_wins_over_the_environment() {
    let out = run_token(
        &["-u", "7", "-s", SECRET, "--json"],
        Some("another-secret-padded-to-32-bytes!"),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let value = stdout_json(&out);
    assert_eq!(
        validate_jwt(value["token"].as_str().unwrap(), SECRET),
        Ok(7)
    );
}

#[test]
fn test_wrong_secret_fails_gateway_validation() {
    let out = run_token(&["-u", "42", "--json"], Some(SECRET));
    let value = stdout_json(&out);

    let result = validate_jwt(value["token"].as_str().unwrap(), "wrong-secret");
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn test_ttl_sets_the_expiry() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let out = run_token(&["-u", "1", "-t", "120", "--json"], Some(SECRET));
    let exp = stdout_json(&out)["exp"].as_u64().unwrap();

    assert!(
        (before + 120..=before + 125).contains(&exp),
        "exp {exp} before {before}"
    );
}

#[test]
fn test_text_mode_prints_only_the_token() {
    let out = run_token(&["-u", "9"], Some(SECRET));
    assert!(out.status.success());

    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert_eq!(validate_jwt(text.trim(), SECRET), Ok(9));
}

#[test]
fn test_missing_secret_exits_2_with_a_json_error() {
    let out = run_token(&["-u", "1", "--json"], None);

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("RSX_GW_JWT_SECRET"));
}
