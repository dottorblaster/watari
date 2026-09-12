use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::*;

const BIN: &str = "watari";

fn watari() -> Command {
    Command::cargo_bin(BIN).expect("the watari binary should be built")
}

#[test]
fn help_prints_usage_and_exits_zero() {
    watari()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("USAGE:"))
        .stdout(predicate::str::contains("--print-config"))
        .stdout(predicate::str::contains("--mirror-clears"))
        .stdout(predicate::str::contains("MIRROR_CLEARS"));
}

#[test]
fn short_help_alias_works() {
    watari()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("USAGE:"));
}

#[test]
fn version_prints_the_crate_version() {
    watari()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")))
        .stdout(predicate::str::starts_with("watari "));
}

#[test]
fn unknown_argument_fails_with_exit_code_two() {
    watari()
        .arg("--definitely-not-a-flag")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("unknown argument"))
        .stderr(predicate::str::contains("--help"));
}

#[test]
fn print_config_defaults_to_colon_zero_and_mirrors_clears() {
    watari()
        .arg("--print-config")
        .env_remove("DISPLAY")
        .env_remove("MIRROR_CLEARS")
        .assert()
        .success()
        .stdout(predicate::str::contains("display=:0\n"))
        .stdout(predicate::str::contains("mirror_clears=true\n"));
}

#[test]
fn print_config_honors_the_environment() {
    watari()
        .arg("--print-config")
        .env("DISPLAY", ":13")
        .env("MIRROR_CLEARS", "off")
        .assert()
        .success()
        .stdout(predicate::str::contains("display=:13\n"))
        .stdout(predicate::str::contains("mirror_clears=false\n"));
}

#[test]
fn print_config_flags_override_the_environment() {
    watari()
        .args(["--print-config", "--display=:42", "--no-mirror-clears"])
        .env("DISPLAY", ":13")
        .env("MIRROR_CLEARS", "true")
        .assert()
        .success()
        .stdout(predicate::str::contains("display=:42\n"))
        .stdout(predicate::str::contains("mirror_clears=false\n"));
}

#[test]
fn print_config_accepts_a_separate_display_value() {
    watari()
        .args(["--print-config", "--display", ":9"])
        .env_remove("DISPLAY")
        .assert()
        .success()
        .stdout(predicate::str::contains("display=:9\n"));
}

#[test]
fn print_config_accepts_an_explicit_mirror_clears_value() {
    watari()
        .args(["--print-config", "--mirror-clears=yes"])
        .env_remove("MIRROR_CLEARS")
        .assert()
        .success()
        .stdout(predicate::str::contains("mirror_clears=true\n"));
}

#[test]
fn daemon_keeps_retrying_when_the_x_server_is_missing() {
    let output = watari()
        .env("DISPLAY", ":231")
        .env("RUST_LOG", "info")
        .timeout(Duration::from_secs(2))
        .output()
        .expect("running the daemon should still produce captured output");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "the daemon is killed by the test timeout, not because it exited on its own"
    );
    assert!(
        stderr.contains("watari: mirroring X CLIPBOARD"),
        "startup banner missing from: {stderr}"
    );
    assert!(
        stderr.contains("X connection unavailable"),
        "missing connection failure log in: {stderr}"
    );
    assert!(
        stderr.contains("reconnecting in"),
        "daemon should announce its backoff retry in: {stderr}"
    );
}
