#![forbid(unsafe_code)]
//! `planeter-i18n-sheet --lang TAG --out FILE` writes the review sheet of a language, generated from the live catalogs (`-` or no
//! `--out`: standard output); `--lang TAG --check FILE` reads a sheet back and exits 1, naming each difference, if it is not exactly
//! what the catalogs say (the counts since the sign-off are printed; they are information, not a failure); `--baseline TAG
//! (web|client)` prints the baseline file of that program's catalog as it stands now (written once, at a sign-off); `--status`
//! prints, for the release notes, one line per signed-off language. Exit 0 ok, 1 drift, 2 usage or input problem.
//!
//! Options: `--glossary FILE` (default: `docs/glossary/TAG.md` of this workspace), `--commit HASH` and `--date YYYY-MM-DD` (default:
//! git's HEAD and today, UTC). Without `--lang`, the one language of the registry that is not the source language.

use std::process::{Command, ExitCode};

use planeter_i18n_sheet::{
    Drift, LANGUAGE_DATA, Program, Stamp, baseline_for, check, civil_date, hard_terms,
    instructions, live_rows, render, state_counts, status_lines,
};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: planeter-i18n-sheet [--lang TAG] [--out FILE|-] [--glossary FILE] [--commit HASH] [--date YYYY-MM-DD]\n       planeter-i18n-sheet [--lang TAG] --check FILE\n       planeter-i18n-sheet --baseline TAG web|client\n       planeter-i18n-sheet --status"
    );
    ExitCode::from(2)
}

fn describe(d: &Drift) -> String {
    match d {
        Drift::EnglishDiffers { key } => format!("`{key}`: the English cell is not the catalog's"),
        Drift::TargetDiffers { key } => {
            format!("`{key}`: the target-language cell is not the catalog's")
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
        Drift::StateDiffers { key } => {
            format!("`{key}`: the \"since sign-off\" cell is not what the baseline says now")
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut lang, mut out, mut check_file, mut glossary, mut commit, mut date) =
        (None, None, None, None, None, None);
    let mut status = false;
    let mut baseline: Option<(String, String)> = None;
    while let Some(a) = args.next() {
        if a == "--status" {
            status = true;
            continue;
        }
        if a == "--baseline" {
            let (Some(tag), Some(program)) = (args.next(), args.next()) else {
                return usage();
            };
            baseline = Some((tag, program));
            continue;
        }
        let slot = match a.as_str() {
            "--lang" => &mut lang,
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
    if status {
        return match status_lines() {
            Ok(lines) => {
                for l in lines {
                    println!("{l}");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("planeter-i18n-sheet: {e}");
                ExitCode::from(2)
            }
        };
    }
    if let Some((tag, program)) = baseline {
        let program = match program.as_str() {
            "web" => Program::Web,
            "client" => Program::Client,
            _ => return usage(),
        };
        return match baseline_for(&tag, program) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("planeter-i18n-sheet: {e}");
                ExitCode::from(2)
            }
        };
    }
    if let Some(file) = check_file {
        let sheet = match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("planeter-i18n-sheet: cannot read {file}: {e}");
                return ExitCode::from(2);
            }
        };
        return match check(&sheet, lang.as_deref()) {
            Ok((tag, drift, rows)) if drift.is_empty() => {
                println!(
                    "{file} ({tag}): every row is what the catalogs say, and every message is there"
                );
                if rows.iter().any(|r| r.state.is_some()) {
                    let (u, c, n) = state_counts(&rows);
                    println!("since the sign-off: {u} unchanged, {c} changed, {n} new");
                }
                ExitCode::SUCCESS
            }
            Ok((_, drift, _)) => {
                for d in &drift {
                    println!("{}", describe(d));
                }
                eprintln!(
                    "planeter-i18n-sheet: {} difference(s) between {file} and the catalogs; regenerate the sheet",
                    drift.len()
                );
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("planeter-i18n-sheet: {e}");
                ExitCode::from(2)
            }
        };
    }
    let tag = match lang {
        Some(t) => t,
        None => {
            let others: Vec<&str> = planeter_i18n::languages::LANGUAGES
                .iter()
                .filter(|l| !l.is_source())
                .map(|l| l.tag)
                .collect();
            match others[..] {
                [one] => one.to_owned(),
                _ => {
                    eprintln!(
                        "planeter-i18n-sheet: --lang is needed (the registry has {} languages besides the source)",
                        others.len()
                    );
                    return usage();
                }
            }
        }
    };
    let rows = match live_rows(&tag) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("planeter-i18n-sheet: {e}");
            return ExitCode::from(2);
        }
    };
    let language = planeter_i18n::languages::by_tag(&tag).expect("live_rows found it");
    let glossary_path = glossary.unwrap_or_else(|| {
        format!(
            "{}/../../docs/glossary/{tag}.md",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let glossary_text = match std::fs::read_to_string(&glossary_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("planeter-i18n-sheet: cannot read {glossary_path}: {e}");
            return ExitCode::from(2);
        }
    };
    let hard = hard_terms(&glossary_text);
    if hard.is_empty() {
        eprintln!(
            "planeter-i18n-sheet: no hard terms found in the \"Terms\" table of {glossary_path}"
        );
        return ExitCode::from(2);
    }
    let Some((_, text, _)) = LANGUAGE_DATA.iter().find(|(t, _, _)| *t == tag) else {
        eprintln!(
            "planeter-i18n-sheet: `{tag}` has no data/{tag}/instructions (add it, and a line in LANGUAGE_DATA)"
        );
        return ExitCode::from(2);
    };
    let instr = match instructions(text, language) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("planeter-i18n-sheet: {e}");
            return ExitCode::from(2);
        }
    };
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
    let sheet = render(&rows, language, &instr, &stamp, &hard);
    match out.as_deref() {
        None | Some("-") => print!("{sheet}"),
        Some(path) => {
            if let Err(e) = std::fs::write(path, &sheet) {
                eprintln!("planeter-i18n-sheet: cannot write {path}: {e}");
                return ExitCode::from(2);
            }
            println!("wrote {path}: {} messages", rows.len());
        }
    }
    ExitCode::SUCCESS
}
