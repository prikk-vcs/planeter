#![forbid(unsafe_code)]
//! `planeter-ja-sheet --out FILE` writes the Japanese review sheet generated from the live catalogs (`-` or no `--out`: standard
//! output); `planeter-ja-sheet --check FILE` reads a sheet back and exits 1, naming each difference, if it is not exactly what the
//! catalogs say. Exit 0 ok, 1 drift, 2 usage or input problem.
//!
//! Options: `--glossary FILE` (default: `docs/glossary.md` of this workspace), `--commit HASH` and `--date YYYY-MM-DD` (default: git's
//! HEAD and today, UTC).

use std::process::{Command, ExitCode};

use planeter_ja_sheet::{Drift, Stamp, check, civil_date, hard_terms, live_rows, render};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: planeter-ja-sheet [--out FILE|-] [--glossary FILE] [--commit HASH] [--date YYYY-MM-DD]\n       planeter-ja-sheet --check FILE"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut out, mut check_file, mut glossary, mut commit, mut date) =
        (None, None, None, None, None);
    while let Some(a) = args.next() {
        let slot = match a.as_str() {
            "--out" => &mut out,
            "--check" => &mut check_file,
            "--glossary" => &mut glossary,
            "--commit" => &mut commit,
            "--date" => &mut date,
            _ => return usage(),
        };
        let Some(v) = args.next() else { return usage() };
        *slot = Some(v);
    }
    if let Some(file) = check_file {
        let sheet = match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("planeter-ja-sheet: cannot read {file}: {e}");
                return ExitCode::from(2);
            }
        };
        return match check(&sheet) {
            Ok(drift) if drift.is_empty() => {
                println!("{file}: every row is what the catalogs say, and every message is there");
                ExitCode::SUCCESS
            }
            Ok(drift) => {
                for d in &drift {
                    println!("{}", describe(d));
                }
                eprintln!(
                    "planeter-ja-sheet: {} difference(s) between {file} and the catalogs; regenerate the sheet",
                    drift.len()
                );
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("planeter-ja-sheet: {e}");
                ExitCode::from(2)
            }
        };
    }
    let rows = match live_rows() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("planeter-ja-sheet: {e}");
            return ExitCode::from(2);
        }
    };
    let glossary_path = glossary
        .unwrap_or_else(|| format!("{}/../../docs/glossary.md", env!("CARGO_MANIFEST_DIR")));
    let glossary_text = match std::fs::read_to_string(&glossary_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("planeter-ja-sheet: cannot read {glossary_path}: {e}");
            return ExitCode::from(2);
        }
    };
    let hard = hard_terms(&glossary_text);
    if hard.is_empty() {
        eprintln!("planeter-ja-sheet: no hard terms found in {glossary_path} (section 5.1)");
        return ExitCode::from(2);
    }
    let stamp = Stamp {
        commit: commit
            .or_else(|| git(&["rev-parse", "HEAD"]))
            .unwrap_or_else(|| "unknown".to_owned()),
        dirty: git(&["status", "--porcelain", "--untracked-files=no"])
            .is_some_and(|s| !s.is_empty()),
        date: date.unwrap_or_else(|| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            civil_date((secs / 86_400) as i64)
        }),
    };
    let sheet = render(&rows, &stamp, &hard);
    match out.as_deref() {
        None | Some("-") => print!("{sheet}"),
        Some(path) => {
            if let Err(e) = std::fs::write(path, &sheet) {
                eprintln!("planeter-ja-sheet: cannot write {path}: {e}");
                return ExitCode::from(2);
            }
            println!("wrote {path}: {} messages", rows.len());
        }
    }
    ExitCode::SUCCESS
}

fn describe(d: &Drift) -> String {
    match d {
        Drift::EnglishDiffers { key } => format!("`{key}`: the English cell is not the catalog's"),
        Drift::JapaneseDiffers { key } => {
            format!("`{key}`: the Japanese cell is not the catalog's")
        }
        Drift::MissingFromSheet { key } => {
            format!("`{key}`: a catalog message the sheet does not have")
        }
        Drift::NotInCatalog { key } => {
            format!("`{key}`: the sheet names a key the catalogs do not have")
        }
        Drift::Duplicate { key } => format!("`{key}`: in the sheet more than once"),
        Drift::WrongGroup {
            key,
            found,
            expected,
        } => {
            format!(
                "`{key}`: in group {} but the catalogs' context puts it in group {}",
                found.letter(),
                expected.letter()
            )
        }
    }
}
