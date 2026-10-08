//! Runs the built binary. Offline paths only: city lookup and input validation never reach
//! NWS, so these tests need no fixture server.
use serde_json::Value;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_weather-bridge"))
        .args(args)
        .env("RUST_LOG", "off")
        .output()
        .expect("binary runs")
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("utf-8 output")
}

#[test]
fn cities_prints_a_table_by_default_and_the_envelope_with_json() {
    let out = run(&["cities", "Seattle, WA"]);
    assert_eq!(out.status.code(), Some(0));
    let table = text(&out.stdout);
    assert!(
        table.contains("ID") && table.contains("Seattle, WA"),
        "{table}"
    );
    assert!(!table.trim_start().starts_with('{'), "{table}");

    let out = run(&["cities", "Seattle, WA", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let body: Value = serde_json::from_slice(&out.stdout).expect("JSON envelope");
    assert_eq!(body["data"][0]["name"], "Seattle");
    assert!(body["meta"]["attribution"].is_array());
}

#[test]
fn ambiguity_prints_every_choice_to_stderr_and_exits_2() {
    for command in ["weather", "hourly", "alerts"] {
        let out = run(&[command, "Franklin"]);
        assert_eq!(out.status.code(), Some(2), "{command}");
        assert!(out.stdout.is_empty(), "{command}: stdout must stay empty");
        let err = text(&out.stderr);
        assert!(err.contains("Several cities share that name"), "{err}");
        assert!(err.contains("1. Franklin, TN (id "), "{err}");
        assert!(err.contains("19. "), "{err}");
        assert!(
            err.contains("--city-id") && err.contains("City, ST"),
            "{err}"
        );
    }
}

#[test]
fn json_errors_go_to_stdout_with_the_same_exit_code() {
    let out = run(&["weather", "Franklin", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&out.stdout).expect("JSON error body");
    assert_eq!(body["errors"][0]["code"], "AMBIGUOUS_CITY");
    assert_eq!(body["errors"][0]["choices"].as_array().unwrap().len(), 19);

    let out = run(&["alerts", "Nowhereville", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["errors"][0]["code"], "CITY_NOT_FOUND");
}

#[test]
fn invalid_input_exits_2_without_network_access() {
    let out = run(&["weather", "--lat", "999", "--lon", "0"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("error:"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(run(&["weather", "--city-id", "1"]).status.code(), Some(2));
    assert_eq!(run(&["cities", "a"]).status.code(), Some(2));
}

#[test]
fn conflicting_or_missing_locations_are_usage_errors() {
    for args in [
        vec!["weather"],
        vec!["weather", "Seattle", "--city-id", "5809844"],
        vec!["hourly", "Seattle", "--lat", "47.6", "--lon", "-122.3"],
        vec!["alerts", "--lat", "47.6"],
        vec!["alerts", "--lon", "-122.3"],
        vec!["weather", "Seattle", "--units", "kelvin"],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn help_documents_exit_codes() {
    let out = run(&["--help"]);
    assert!(text(&out.stdout).contains("Exit codes"));
}

#[cfg(unix)]
#[test]
fn closed_stdout_preserves_the_command_exit_code_without_panicking() {
    use std::{os::fd::OwnedFd, os::unix::net::UnixStream};

    for (args, expected) in [
        (vec!["cities", "Springfield"], 0),
        (vec!["cities", "Springfield", "--json"], 0),
        (vec!["cities", "a", "--json"], 2),
    ] {
        // Close the consumer before spawning so the outcome cannot race the writer.
        let (reader, writer) = UnixStream::pair().unwrap();
        drop(reader);
        let out = Command::new(env!("CARGO_BIN_EXE_weather-bridge"))
            .args(&args)
            .env("RUST_LOG", "off")
            .stdout(OwnedFd::from(writer))
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(expected),
            "{args:?}: {}",
            text(&out.stderr)
        );
        assert!(out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
    }
}
