use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use askbridge_core::sha256_file;

const MAX_DESKTOP_WORKING_SET_BYTES: f64 = 35.0 * 1024.0 * 1024.0;

#[derive(Debug)]
pub(crate) struct PerformanceReportOptions {
    desktop_report_path: PathBuf,
    chrome_report_path: PathBuf,
    timings_report_path: PathBuf,
    executable_path: PathBuf,
    expected_chrome_profile_path: PathBuf,
    minimum_desktop_duration_seconds: f64,
    minimum_chrome_duration_seconds: f64,
}

impl PerformanceReportOptions {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut desktop_report_path = None;
        let mut chrome_report_path = None;
        let mut timings_report_path = None;
        let mut executable_path = None;
        let mut expected_chrome_profile_path = None;
        let mut minimum_desktop_duration_seconds = 300.0;
        let mut minimum_chrome_duration_seconds = 300.0;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--desktop-report-path" => desktop_report_path = Some(value.into()),
                "--chrome-report-path" => chrome_report_path = Some(value.into()),
                "--timings-report-path" => timings_report_path = Some(value.into()),
                "--executable-path" => executable_path = Some(value.into()),
                "--expected-chrome-profile-path" => {
                    expected_chrome_profile_path = Some(value.into())
                }
                "--minimum-desktop-duration-seconds" => {
                    minimum_desktop_duration_seconds = parse_bounded(&flag, &value, 1.0, 3600.0)?
                }
                "--minimum-chrome-duration-seconds" => {
                    minimum_chrome_duration_seconds = parse_bounded(&flag, &value, 1.0, 1800.0)?
                }
                _ => return Err(format!("unknown option '{flag}'")),
            }
        }
        Ok(Self {
            desktop_report_path: required_path(
                desktop_report_path,
                "DesktopReportPath is required for final performance validation.",
            )?,
            chrome_report_path: required_path(
                chrome_report_path,
                "ChromeReportPath is required for final performance validation.",
            )?,
            timings_report_path: required_path(
                timings_report_path,
                "TimingsReportPath is required for final performance validation.",
            )?,
            executable_path: required_path(
                executable_path,
                "ExecutablePath is required for final performance validation.",
            )?,
            expected_chrome_profile_path: required_path(
                expected_chrome_profile_path,
                "ExpectedChromeProfilePath is required for final performance validation.",
            )?,
            minimum_desktop_duration_seconds,
            minimum_chrome_duration_seconds,
        })
    }
}

pub(crate) fn validate_performance_report(
    options: &PerformanceReportOptions,
) -> Result<(), String> {
    let desktop_path = required_file(&options.desktop_report_path, "DesktopReportPath")?;
    let desktop = read_json(&desktop_path)?;
    validate_measured_at(&desktop, "Desktop report")?;
    let desktop_hash = required_string(&desktop, &["executable_sha256"], "executable_sha256")?;
    require_samples(&desktop, "Desktop report")?;
    require_number_greater_than(&desktop, "cold_start_ms", 0.0)?;
    require_number_at_least(
        &desktop,
        "actual_duration_seconds",
        options.minimum_desktop_duration_seconds,
    )?;
    require_number_at_most(&desktop, "idle_cpu_percent_machine", 0.2)?;

    let desktop_working_set = required_number(
        &desktop,
        &["working_set_max_bytes", "working_set_bytes_max"],
        "desktop working set max",
    )?;
    if desktop_working_set <= 0.0 || desktop_working_set > MAX_DESKTOP_WORKING_SET_BYTES {
        return Err(format!(
            "desktop working set max is outside the acceptance bound: {desktop_working_set}"
        ));
    }
    let desktop_external_connections = required_i64(
        &desktop,
        &[
            "external_tcp_connection_count_max",
            "external_tcp_connection_count",
        ],
        "desktop external TCP connection count",
    )?;
    if desktop_external_connections != 0 {
        return Err(format!(
            "desktop external TCP connection count is {desktop_external_connections}, expected 0."
        ));
    }
    let desktop_process_count = required_i64(
        &desktop,
        &["process_count_max", "process_count"],
        "desktop process count",
    )?;
    if desktop_process_count != 1 {
        return Err(format!(
            "desktop process count is {desktop_process_count}, expected 1."
        ));
    }

    let executable = required_file(&options.executable_path, "ExecutablePath")?;
    validate_report_executable(&desktop, &executable, "Desktop report")?;
    let actual_hash = sha256(&executable)?;
    if !actual_hash.eq_ignore_ascii_case(&desktop_hash) {
        return Err(format!(
            "Desktop report executable hash is stale. report={desktop_hash} actual={actual_hash}"
        ));
    }

    let chrome_path = required_file(&options.chrome_report_path, "ChromeReportPath")?;
    let chrome = read_json(&chrome_path)?;
    validate_measured_at(&chrome, "Chrome report")?;
    validate_report_executable(&chrome, &executable, "Chrome report")?;
    let expected_profile = absolute_normalized(
        &options.expected_chrome_profile_path,
        "ExpectedChromeProfilePath",
    )?;
    let reported_profile = PathBuf::from(required_string(
        &chrome,
        &["profile_path"],
        "Chrome profile path",
    )?);
    let reported_profile = absolute_normalized(&reported_profile, "Chrome report profile_path")?;
    if !same_path(&reported_profile, &expected_profile) {
        return Err(format!(
            "Chrome report profile path does not match the expected AskBridge profile. report={} expected={}",
            reported_profile.display(),
            expected_profile.display()
        ));
    }
    let chrome_hash = required_string(&chrome, &["executable_sha256"], "executable_sha256")?;
    if !actual_hash.eq_ignore_ascii_case(&chrome_hash) {
        return Err(format!(
            "Chrome report executable hash is stale. report={chrome_hash} actual={actual_hash}"
        ));
    }
    require_samples(&chrome, "Chrome report")?;
    require_number_at_least(
        &chrome,
        "actual_duration_seconds",
        options.minimum_chrome_duration_seconds,
    )?;
    require_number_greater_than(&chrome, "working_set_average_bytes", 0.0)?;
    require_number_greater_than(&chrome, "process_count_max", 0.0)?;

    let timings_path = required_file(&options.timings_report_path, "TimingsReportPath")?;
    let timings = read_json(&timings_path)?;
    if timings.get("auto_submit").and_then(Value::as_bool) != Some(false)
        || timings
            .get("managed_browser_closed")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(
            "Preparation timings must record auto_submit=false and managed_browser_closed=true."
                .to_owned(),
        );
    }
    required_string(&timings, &["provider"], "Preparation timings provider")?;
    for name in [
        "measured_at_unix_ms",
        "browser_launch_ms",
        "first_preparation_ms",
        "continuous_preparation_ms",
    ] {
        require_number_greater_than(&timings, name, 0.0)?;
    }
    Ok(())
}

fn parse_bounded(label: &str, value: &str, minimum: f64, maximum: f64) -> Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| format!("{label} must be a number"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{label} must be between {minimum} and {maximum}"));
    }
    Ok(value)
}

fn required_path(path: Option<PathBuf>, message: &str) -> Result<PathBuf, String> {
    path.filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| message.to_owned())
}

fn required_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = absolute_normalized(path, label)?;
    if !path.is_file() {
        return Err(format!("{label} does not exist: {}", path.display()));
    }
    Ok(path)
}

fn absolute_normalized(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an explicit absolute path."));
    }
    path.canonicalize().or_else(|_| {
        let parent = path
            .parent()
            .ok_or_else(|| format!("{label} could not be normalized: {}", path.display()))?;
        let parent = parent
            .canonicalize()
            .map_err(|_| format!("{label} could not be normalized: {}", path.display()))?;
        let name = path
            .file_name()
            .ok_or_else(|| format!("{label} could not be normalized: {}", path.display()))?;
        Ok(parent.join(name))
    })
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice(bytes).map_err(|_| format!("invalid JSON: {}", path.display()))
}

fn validate_measured_at(report: &Value, label: &str) -> Result<(), String> {
    let measured_at = required_string(report, &["measured_at"], &format!("{label} measured_at"))?;
    if is_iso_8601_timestamp(&measured_at) {
        Ok(())
    } else {
        Err(format!(
            "{label} measured_at must be an ISO 8601 timestamp."
        ))
    }
}

fn validate_report_executable(report: &Value, expected: &Path, label: &str) -> Result<(), String> {
    let reported = required_string(report, &["executable"], &format!("{label} executable path"))?;
    let reported = absolute_normalized(Path::new(&reported), &format!("{label} executable path"))?;
    if !same_path(&reported, expected) {
        return Err(format!(
            "{label} executable path does not match the expected Release EXE. report={} expected={}",
            reported.display(),
            expected.display()
        ));
    }
    Ok(())
}

fn require_samples(report: &Value, label: &str) -> Result<(), String> {
    if report
        .get("samples")
        .and_then(Value::as_array)
        .is_none_or(|samples| samples.len() < 2)
    {
        return Err(format!("{label} must include at least two samples."));
    }
    Ok(())
}

fn require_number_at_most(report: &Value, name: &str, maximum: f64) -> Result<(), String> {
    let value = required_number(report, &[name], name)?;
    if value > maximum {
        return Err(format!("{name} is {value}, expected at most {maximum}."));
    }
    Ok(())
}

fn require_number_greater_than(report: &Value, name: &str, minimum: f64) -> Result<(), String> {
    let value = required_number(report, &[name], name)?;
    if value <= minimum {
        return Err(format!(
            "{name} is {value}, expected greater than {minimum}."
        ));
    }
    Ok(())
}

fn require_number_at_least(report: &Value, name: &str, minimum: f64) -> Result<(), String> {
    let value = required_number(report, &[name], name)?;
    if value < minimum {
        return Err(format!("{name} is {value}, expected at least {minimum}."));
    }
    Ok(())
}

fn required_number(report: &Value, names: &[&str], label: &str) -> Result<f64, String> {
    names
        .iter()
        .find_map(|name| report.get(*name).and_then(Value::as_f64))
        .ok_or_else(|| format!("{label} is missing."))
}

fn required_i64(report: &Value, names: &[&str], label: &str) -> Result<i64, String> {
    names
        .iter()
        .find_map(|name| report.get(*name).and_then(Value::as_i64))
        .ok_or_else(|| format!("{label} is missing."))
}

fn required_string(report: &Value, names: &[&str], label: &str) -> Result<String, String> {
    names
        .iter()
        .find_map(|name| report.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} is missing."))
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn sha256(path: &Path) -> Result<String, String> {
    sha256_file(path).map_err(|error| format!("reading {}: {error}", path.display()))
}

fn is_iso_8601_timestamp(value: &str) -> bool {
    let Some((date, time_and_offset)) = value.split_once('T') else {
        return false;
    };
    let date_parts: Vec<_> = date.split('-').collect();
    if date_parts.len() != 3 {
        return false;
    }
    let Some(year) = parse_fixed_number(date_parts[0], 4, 1, 9999) else {
        return false;
    };
    let Some(month) = parse_fixed_number(date_parts[1], 2, 1, 12) else {
        return false;
    };
    let Some(day) = parse_fixed_number(date_parts[2], 2, 1, days_in_month(year, month)) else {
        return false;
    };
    debug_assert!(day > 0);
    let (time, offset) = if let Some(time) = time_and_offset.strip_suffix('Z') {
        (time, None)
    } else {
        let Some(offset_search) = time_and_offset.get(1..) else {
            return false;
        };
        let Some(relative_index) = offset_search.rfind(['+', '-']) else {
            return false;
        };
        let index = relative_index + 1;
        (
            &time_and_offset[..index],
            Some(&time_and_offset[index + 1..]),
        )
    };
    if let Some(offset) = offset {
        let parts: Vec<_> = offset.split(':').collect();
        if parts.len() != 2
            || !valid_fixed_number(parts[0], 2, 0, 23)
            || !valid_fixed_number(parts[1], 2, 0, 59)
        {
            return false;
        }
    }
    let time_parts: Vec<_> = time.split(':').collect();
    if time_parts.len() != 3
        || !valid_fixed_number(time_parts[0], 2, 0, 23)
        || !valid_fixed_number(time_parts[1], 2, 0, 59)
    {
        return false;
    }
    let (seconds, fraction) = time_parts[2]
        .split_once('.')
        .map_or((time_parts[2], None), |(seconds, fraction)| {
            (seconds, Some(fraction))
        });
    valid_fixed_number(seconds, 2, 0, 59)
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty()
                && fraction.len() <= 9
                && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn valid_fixed_number(value: &str, width: usize, minimum: u32, maximum: u32) -> bool {
    parse_fixed_number(value, width, minimum, maximum).is_some()
}

fn parse_fixed_number(value: &str, width: usize, minimum: u32, maximum: u32) -> Option<u32> {
    if value.len() != width || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|value| (minimum..=maximum).contains(value))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_json::json;
    use tempfile::TempDir;

    use super::{PerformanceReportOptions, sha256, validate_performance_report};

    struct Fixture {
        _root: TempDir,
        options: PerformanceReportOptions,
    }

    impl Fixture {
        fn valid() -> Self {
            let root = tempfile::tempdir().expect("tempdir");
            let executable = root.path().join("askbridge.exe");
            fs::write(&executable, b"validator-fixture").expect("executable");
            let hash = sha256(&executable).expect("hash");
            let profile = root.path().join("BrowserProfile");
            fs::create_dir(&profile).expect("profile");
            let desktop = root.path().join("desktop.json");
            let chrome = root.path().join("chrome.json");
            let timings = root.path().join("timings.json");
            write_json(
                &desktop,
                &json!({
                    "measured_at": "2026-08-12T00:00:00+08:00",
                    "executable": executable,
                    "executable_sha256": hash,
                    "cold_start_ms": 120.5,
                    "actual_duration_seconds": 300,
                    "idle_cpu_percent_machine": 0.05,
                    "working_set_max_bytes": 15 * 1024 * 1024,
                    "external_tcp_connection_count_max": 0,
                    "process_count_max": 1,
                    "samples": [{"sample": 1}, {"sample": 2}]
                }),
            );
            write_json(
                &chrome,
                &json!({
                    "measured_at": "2026-08-12T00:00:00+08:00",
                    "profile_path": profile,
                    "executable": executable,
                    "executable_sha256": hash,
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
            Self {
                options: PerformanceReportOptions {
                    desktop_report_path: desktop,
                    chrome_report_path: chrome,
                    timings_report_path: timings,
                    executable_path: executable,
                    expected_chrome_profile_path: profile,
                    minimum_desktop_duration_seconds: 300.0,
                    minimum_chrome_duration_seconds: 300.0,
                },
                _root: root,
            }
        }
    }

    #[test]
    fn accepts_matching_complete_reports() {
        let fixture = Fixture::valid();
        validate_performance_report(&fixture.options).expect("valid reports");
    }

    #[test]
    fn rejects_stale_executable_hash() {
        let fixture = Fixture::valid();
        fs::write(&fixture.options.executable_path, b"changed").expect("change executable");
        let error = validate_performance_report(&fixture.options).expect_err("stale hash");
        assert!(error.starts_with("Desktop report executable hash is stale."));
    }

    #[test]
    fn rejects_auto_submit_or_open_managed_browser() {
        let fixture = Fixture::valid();
        write_json(
            &fixture.options.timings_report_path,
            &json!({
                "measured_at_unix_ms": 1,
                "provider": "chatgpt",
                "auto_submit": true,
                "managed_browser_closed": false,
                "browser_launch_ms": 1,
                "first_preparation_ms": 1,
                "continuous_preparation_ms": 1
            }),
        );
        let error = validate_performance_report(&fixture.options).expect_err("safety flags");
        assert_eq!(
            error,
            "Preparation timings must record auto_submit=false and managed_browser_closed=true."
        );
    }

    #[test]
    fn rejects_profile_from_another_location() {
        let fixture = Fixture::valid();
        let other = fixture._root.path().join("OtherProfile");
        fs::create_dir(&other).expect("other profile");
        let chrome = serde_json::from_slice::<serde_json::Value>(
            &fs::read(&fixture.options.chrome_report_path).expect("chrome report"),
        )
        .expect("chrome JSON");
        let mut chrome = chrome.as_object().expect("object").clone();
        chrome.insert("profile_path".to_owned(), json!(other));
        write_json(
            &fixture.options.chrome_report_path,
            &serde_json::Value::Object(chrome),
        );
        let error = validate_performance_report(&fixture.options).expect_err("wrong profile");
        assert!(error.starts_with(
            "Chrome report profile path does not match the expected AskBridge profile."
        ));
    }

    #[test]
    fn rejects_relative_final_evidence_paths() {
        let mut fixture = Fixture::valid();
        fixture.options.desktop_report_path = "relative-desktop.json".into();
        let error = validate_performance_report(&fixture.options).expect_err("relative path");
        assert_eq!(
            error,
            "DesktopReportPath must be an explicit absolute path."
        );
    }

    #[test]
    fn rejects_unsafe_desktop_bounds() {
        let fixture = Fixture::valid();
        edit_json(&fixture.options.desktop_report_path, |report| {
            report["idle_cpu_percent_machine"] = json!(0.21);
        });
        let error = validate_performance_report(&fixture.options).expect_err("desktop CPU");
        assert!(error.starts_with("idle_cpu_percent_machine is 0.21"));

        let fixture = Fixture::valid();
        edit_json(&fixture.options.desktop_report_path, |report| {
            report["working_set_max_bytes"] = json!(36 * 1024 * 1024);
        });
        let error = validate_performance_report(&fixture.options).expect_err("working set");
        assert!(error.starts_with("desktop working set max is outside the acceptance bound:"));
    }

    #[test]
    fn rejects_missing_desktop_external_connection_evidence() {
        let fixture = Fixture::valid();
        edit_json(&fixture.options.desktop_report_path, |report| {
            report
                .as_object_mut()
                .expect("object")
                .remove("external_tcp_connection_count_max");
        });
        let error = validate_performance_report(&fixture.options).expect_err("missing evidence");
        assert_eq!(error, "desktop external TCP connection count is missing.");
    }

    #[test]
    fn rejects_invalid_timestamp_and_under_duration_reports() {
        let fixture = Fixture::valid();
        edit_json(&fixture.options.desktop_report_path, |report| {
            report["measured_at"] = json!("not-a-timestamp");
        });
        let error = validate_performance_report(&fixture.options).expect_err("timestamp");
        assert_eq!(
            error,
            "Desktop report measured_at must be an ISO 8601 timestamp."
        );

        let fixture = Fixture::valid();
        edit_json(&fixture.options.chrome_report_path, |report| {
            report["actual_duration_seconds"] = json!(299);
        });
        let error = validate_performance_report(&fixture.options).expect_err("duration");
        assert!(error.starts_with("actual_duration_seconds is 299"));
    }

    #[test]
    fn rejects_incomplete_preparation_timings() {
        let fixture = Fixture::valid();
        edit_json(&fixture.options.timings_report_path, |report| {
            report["continuous_preparation_ms"] = json!(0);
        });
        let error = validate_performance_report(&fixture.options).expect_err("timing");
        assert!(error.starts_with("continuous_preparation_ms is 0"));

        let fixture = Fixture::valid();
        edit_json(&fixture.options.timings_report_path, |report| {
            report["provider"] = json!("");
        });
        let error = validate_performance_report(&fixture.options).expect_err("provider");
        assert_eq!(error, "Preparation timings provider is missing.");
    }

    #[test]
    fn rejects_missing_or_stale_chrome_executable_hash() {
        let fixture = Fixture::valid();
        edit_json(&fixture.options.chrome_report_path, |report| {
            report
                .as_object_mut()
                .expect("object")
                .remove("executable_sha256");
        });
        let error = validate_performance_report(&fixture.options).expect_err("missing hash");
        assert_eq!(error, "executable_sha256 is missing.");

        let fixture = Fixture::valid();
        edit_json(&fixture.options.chrome_report_path, |report| {
            report["executable_sha256"] = json!("0".repeat(64));
        });
        let error = validate_performance_report(&fixture.options).expect_err("stale hash");
        assert!(error.starts_with("Chrome report executable hash is stale."));
    }

    #[test]
    fn accepts_utf8_bom_from_powershell_reports() {
        let fixture = Fixture::valid();
        let path = &fixture.options.desktop_report_path;
        let bytes = fs::read(path).expect("desktop report");
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend(bytes);
        fs::write(path, with_bom).expect("BOM report");
        validate_performance_report(&fixture.options).expect("PowerShell UTF-8 report");
    }

    #[test]
    fn rejects_impossible_or_truncated_timestamps_without_panicking() {
        for timestamp in ["2026-02-31T00:00:00+08:00", "2026-08-19T"] {
            let fixture = Fixture::valid();
            edit_json(&fixture.options.desktop_report_path, |report| {
                report["measured_at"] = json!(timestamp);
            });
            let error = validate_performance_report(&fixture.options)
                .expect_err("invalid timestamp must be rejected");
            assert_eq!(
                error,
                "Desktop report measured_at must be an ISO 8601 timestamp."
            );
        }
    }

    #[test]
    fn parser_requires_every_final_evidence_path() {
        let error =
            PerformanceReportOptions::parse(Vec::<String>::new()).expect_err("missing evidence");
        assert_eq!(
            error,
            "DesktopReportPath is required for final performance validation."
        );
    }

    fn write_json(path: &Path, value: &serde_json::Value) {
        fs::write(path, serde_json::to_vec_pretty(value).expect("JSON")).expect("write JSON");
    }

    fn edit_json(path: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut value = serde_json::from_slice::<serde_json::Value>(
            &fs::read(path).expect("read JSON for edit"),
        )
        .expect("JSON for edit");
        edit(&mut value);
        write_json(path, &value);
    }
}
