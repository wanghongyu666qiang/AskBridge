mod performance_report;
mod sha256;

use std::{env, process::ExitCode};

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
    "usage: cargo xtask validate-performance-report \\
  --desktop-report-path <absolute-path> \\
  --chrome-report-path <absolute-path> \\
  --timings-report-path <absolute-path> \\
  --executable-path <absolute-path> \\
  --expected-chrome-profile-path <absolute-path> \\
  [--minimum-desktop-duration-seconds <1..3600>] \\
  [--minimum-chrome-duration-seconds <1..1800>]"
        .to_owned()
}
