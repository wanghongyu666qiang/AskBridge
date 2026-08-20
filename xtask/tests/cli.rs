use std::{fs, path::Path, process::Command};

use serde_json::json;

#[test]
fn help_command_exercises_binary_dispatch() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("help")
        .output()
        .expect("run xtask help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    assert!(stdout.contains("cargo xtask validate-package-artifacts"));
    assert!(stdout.contains("cargo xtask validate-performance-report"));
}

#[test]
fn validate_package_artifacts_reports_missing_evidence_through_real_cli() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("validate-package-artifacts")
        .output()
        .expect("run package validator failure");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("ExpectedVersion is required for final package artifact validation."));
}

#[test]
fn unknown_command_returns_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("not-a-command")
        .output()
        .expect("run xtask failure");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("unknown xtask command 'not-a-command'"));
}

#[test]
fn validate_performance_report_runs_through_real_cli() {
    let root = tempfile::tempdir().expect("tempdir");
    let executable = root.path().join("askbridge.exe");
    fs::write(&executable, b"abc").expect("executable");
    let executable_hash = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
    let profile = root.path().join("BrowserProfile");
    fs::create_dir(&profile).expect("profile");
    let desktop = root.path().join("desktop.json");
    let chrome = root.path().join("chrome.json");
    let timings = root.path().join("timings.json");
    write_json(
        &desktop,
        &json!({
            "measured_at": "2026-08-19T00:00:00+08:00",
            "executable": executable,
            "executable_sha256": executable_hash,
            "cold_start_ms": 100,
            "actual_duration_seconds": 300,
            "idle_cpu_percent_machine": 0.1,
            "working_set_max_bytes": 20 * 1024 * 1024,
            "external_tcp_connection_count_max": 0,
            "process_count_max": 1,
            "samples": [{"sample": 1}, {"sample": 2}]
        }),
    );
    write_json(
        &chrome,
        &json!({
            "measured_at": "2026-08-19T00:00:00+08:00",
            "profile_path": profile,
            "executable": executable,
            "executable_sha256": executable_hash,
            "actual_duration_seconds": 300,
            "working_set_average_bytes": 800 * 1024 * 1024_u64,
            "process_count_max": 8,
            "samples": [{"sample": 1}, {"sample": 2}]
        }),
    );
    write_json(
        &timings,
        &json!({
            "measured_at_unix_ms": 1_786_304_522_013_i64,
            "provider": "chatgpt",
            "auto_submit": false,
            "managed_browser_closed": true,
            "browser_launch_ms": 600,
            "first_preparation_ms": 6000,
            "continuous_preparation_ms": 3800
        }),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "validate-performance-report",
            "--desktop-report-path",
            path(&desktop),
            "--chrome-report-path",
            path(&chrome),
            "--timings-report-path",
            path(&timings),
            "--executable-path",
            path(&executable),
            "--expected-chrome-profile-path",
            path(&profile),
        ])
        .output()
        .expect("run validator CLI");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .expect("UTF-8 stdout")
            .contains("Performance reports are internally valid.")
    );
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON")).expect("write JSON");
}

fn path(path: &Path) -> &str {
    path.to_str().expect("Unicode test path")
}
