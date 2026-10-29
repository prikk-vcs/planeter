#![forbid(unsafe_code)]
//! `planeter-glibc-check BINARY BASE` — BASE is the image's glibc: a version (`2.41`) or the first line of its `ldd --version`.
//! Exit 0 when the image's glibc defines every version the binary strictly needs, 1 when it does not (each is named), 2 on a usage
//! problem or when `readelf` cannot be run.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [binary, base] = &args[..] else {
        eprintln!(
            "usage: planeter-glibc-check BINARY BASE   (BASE: a glibc version or the first line of `ldd --version`)"
        );
        return ExitCode::from(2);
    };
    let Some(base) = planeter_glibc_check::base_version(base) else {
        eprintln!("planeter-glibc-check: cannot read a glibc version from {base:?}");
        return ExitCode::from(2);
    };
    let out = match Command::new("readelf").args(["-V", binary]).output() {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            eprintln!(
                "planeter-glibc-check: readelf failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("planeter-glibc-check: cannot run readelf (binutils): {e}");
            return ExitCode::from(2);
        }
    };
    let needs = planeter_glibc_check::parse_needs(&String::from_utf8_lossy(&out.stdout));
    if needs.is_empty() {
        eprintln!(
            "planeter-glibc-check: no GLIBC_ version need found in {binary}: not a dynamically linked glibc binary, or readelf printed nothing"
        );
        return ExitCode::from(2);
    }
    let verdict = planeter_glibc_check::check(&needs, &base);
    print!("{verdict}");
    if verdict.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
