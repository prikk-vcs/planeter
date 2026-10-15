#![forbid(unsafe_code)]
//! `planeter-pin-check [ROOT]` — exit 0 when the build chain is fully pinned, 1 when any mutable
//! reference is found (each is named as `file:line: message`), 2 on a usage or I/O problem.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    if args.next().is_some() {
        eprintln!("usage: planeter-pin-check [ROOT]");
        return ExitCode::from(2);
    }
    match planeter_pin_check::check(&root) {
        Ok(report) => {
            print!("{report}");
            if report.findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("planeter-pin-check: {e}");
            ExitCode::from(2)
        }
    }
}
