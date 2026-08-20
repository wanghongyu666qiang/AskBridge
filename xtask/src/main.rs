mod package_artifacts;
mod performance_report;
mod sha256;

use std::{env, process::ExitCode};

use package_artifacts::{PackageArtifactOptions, validate_package_artifacts};
use performance_report::{PerformanceReportOptions, validate_performance_report};

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(usage());
    };
    match command.as_str() {
        "validate-package-artifacts" => {
            let options = PackageArtifactOptions::parse(args)?;
            validate_package_artifacts(&options)?;
            println!("Package artifact validation passed.");
            Ok(())
        }
        "validate-performance-report" => {
            let options = PerformanceReportOptions::parse(args)?;
            validate_performance_report(&options)?;
            println!("Performance reports are internally valid.");
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            Ok(())
        }
        _ => Err(format!("unknown xtask command '{command}'\n\n{}", usage())),
    }
}

fn usage() -> String {
    "usage:
  cargo xtask validate-package-artifacts \\
  --artifact-root <absolute-path> \\
  --expected-version <version> \\
  --expected-release-exe-path <absolute-path> \\
  --expected-source-root <absolute-path> \\
  [--max-release-exe-bytes <bytes>] \\
  [--max-setup-bytes <bytes>] \\
  [--max-static-resource-bytes <bytes>]

  cargo xtask validate-performance-report \\
  --desktop-report-path <absolute-path> \\
  --chrome-report-path <absolute-path> \\
  --timings-report-path <absolute-path> \\
  --executable-path <absolute-path> \\
  --expected-chrome-profile-path <absolute-path> \\
  [--minimum-desktop-duration-seconds <1..3600>] \\
  [--minimum-chrome-duration-seconds <1..1800>]"
        .to_owned()
}
