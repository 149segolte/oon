// SPDX-License-Identifier: MPL-2.0

use std::fs;
use std::process::Command;

use tempfile::tempdir;

#[test]
fn cli_emits_json_and_separates_diagnostics() {
    let binary = env!("CARGO_BIN_EXE_oon");
    let output = Command::new(binary)
        .args([
            "--schema",
            "tests/fixtures/config.sch.oon",
            "tests/fixtures/workstation.oon",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8(output.stdout).unwrap().ends_with("\n"));
    let invalid = Command::new(binary).arg("--wat").output().unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert!(!invalid.stderr.is_empty());
    let _ = fs::metadata(binary).unwrap();
}

#[test]
fn cli_preserves_explicit_order_and_sorts_directory_entries() {
    let binary = env!("CARGO_BIN_EXE_oon");
    let directory = tempdir().unwrap();
    let schema = directory.path().join("config.sch.oon");
    let a = directory.path().join("a.oon");
    let b = directory.path().join("b.oon");
    let excluded = directory.path().join("z.sch.oon");
    fs::write(&schema, "schema config = { values = list<string>; };").unwrap();
    fs::write(
        &a,
        "schema = \"config\"; overlay a = { merge .values = [\"a\";]; };",
    )
    .unwrap();
    fs::write(
        &b,
        "schema = \"config\"; overlay b = { merge .values = [\"b\";]; };",
    )
    .unwrap();
    fs::write(&excluded, "this must not be loaded").unwrap();

    let explicit = Command::new(binary)
        .arg("--schema")
        .arg(&schema)
        .arg(&b)
        .arg(&a)
        .output()
        .unwrap();
    assert!(explicit.status.success());
    assert!(
        String::from_utf8(explicit.stdout)
            .unwrap()
            .contains("[\n    \"b\",\n    \"a\"\n  ]")
    );

    let sorted = Command::new(binary)
        .arg("--schema")
        .arg(&schema)
        .arg("--overlays-dir")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(
        sorted.status.success(),
        "{}",
        String::from_utf8_lossy(&sorted.stderr)
    );
    assert!(
        String::from_utf8(sorted.stdout)
            .unwrap()
            .contains("[\n    \"a\",\n    \"b\"\n  ]")
    );
}

#[test]
fn cli_loads_an_initial_json_value_and_accepts_option_ordering() {
    let binary = env!("CARGO_BIN_EXE_oon");
    let directory = tempdir().unwrap();
    let schema = directory.path().join("config.sch.oon");
    let initial = directory.path().join("initial.json");
    let overlay = directory.path().join("update.oon");
    fs::write(
        &schema,
        "schema config = { x = int; y = int; label? = string; };",
    )
    .unwrap();
    fs::write(&initial, r#"{"x":10}"#).unwrap();
    fs::write(
        &overlay,
        "schema = \"config\"; overlay update = { .y = .x; reset .x; };",
    )
    .unwrap();

    let output = Command::new(binary)
        .arg("--schema")
        .arg(&schema)
        .arg(&overlay)
        .arg("--initial-value")
        .arg(&initial)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "{\n  \"x\": 0,\n  \"y\": 10\n}\n"
    );
}

#[test]
fn cli_reports_initial_value_and_usage_failures_cleanly() {
    let binary = env!("CARGO_BIN_EXE_oon");
    let directory = tempdir().unwrap();
    let schema = directory.path().join("config.sch.oon");
    let malformed = directory.path().join("malformed.json");
    let invalid = directory.path().join("invalid.json");
    fs::write(&schema, "schema config = { value = int; };").unwrap();
    fs::write(&malformed, "{").unwrap();
    fs::write(&invalid, r#"{"value":"wrong"}"#).unwrap();

    for path in [&malformed, &invalid] {
        let output = Command::new(binary)
            .arg("--schema")
            .arg(&schema)
            .arg("--initial-value")
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }

    let duplicate = Command::new(binary)
        .arg("--schema")
        .arg(&schema)
        .arg("--initial-value")
        .arg(&invalid)
        .arg("--initial-value")
        .arg(&invalid)
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(duplicate.stdout.is_empty());

    let missing = Command::new(binary)
        .arg("--schema")
        .arg(&schema)
        .arg("--initial-value")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));

    let missing_before_option = Command::new(binary)
        .arg("--schema")
        .arg(&schema)
        .arg("--initial-value")
        .arg("--overlays-dir")
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(missing_before_option.status.code(), Some(2));
}
