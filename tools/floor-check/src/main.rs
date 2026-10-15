#![forbid(unsafe_code)]
//! `planeter-floor-check [ROOT]` — exit 0 when the three prikk floors agree, 1 when they do not
//! (each file and value is named), 2 on a usage or I/O problem.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    if args.next().is_some() {
        eprintln!("usage: planeter-floor-check [ROOT]");
        return ExitCode::from(2);
    }
    match planeter_floor_check::check(&root) {
        Ok(report) => {
            print!("{report}");
            if report.agrees() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("planeter-floor-check: {e}");
            ExitCode::from(2)
        }
    }
}
