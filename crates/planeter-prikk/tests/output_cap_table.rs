//! IS-55: the cap table (`output_cap::TABLE`) is held to its own rules: every verb the drivers use has a measured row, every limit is at
//! least twice the largest legitimate output recorded for its row, `log --limit 100000` admits what a 100,000-block log is, and a
//! verb nobody measured falls to a finite general limit.

use planeter_prikk::output_cap::{GENERAL, STDERR, TABLE, for_args, is_in_table, rule_of, verb_of};

const MIB: u64 = 1 << 20;

fn limit(args: &[&str]) -> u64 {
    for_args(args).stdout
}

#[test]
fn every_row_is_at_least_twice_its_largest_legitimate_output() {
    for rule in TABLE {
        // The rows whose limit depends on the arguments are held to their margin below, per call.
        let probe: Vec<&str> = match rule.verbs[0] {
            "log" => vec!["log", "--format", "json"],
            "cat" => vec!["cat", "--path", "x"],
            v => v.split(' ').collect(),
        };
        let got = limit(&probe);
        assert!(
            got >= 2 * rule.largest,
            "{:?}: limit {got} is below twice {} B ({})",
            rule.verbs,
            rule.largest,
            rule.measured
        );
        assert!(rule.largest > 0 && !rule.measured.is_empty() && !rule.why.is_empty());
    }
}

#[test]
fn a_log_of_a_hundred_thousand_blocks_is_admitted_and_a_cat_is_bounded_by_what_was_asked() {
    // 635 B a block measured (200 blocks: 127,137 B), so 63.5 MB for the observation's LOG_BLOCK_LIMIT; twice that must fit.
    let observation = limit(&[
        "log",
        "--format",
        "json",
        "--ref",
        "heads/main",
        "--limit",
        "100000",
    ]);
    assert!(observation >= 2 * 63_500_000, "{observation}");
    // The observation of proposals (IS-55 ruling): `--limit 10000`, 6.35 MB, a cap near 23.7 MB.
    let window = limit(&[
        "log",
        "--format",
        "json",
        "--ref",
        "heads/main",
        "--limit",
        "10000",
    ]);
    assert!((2 * 6_350_000..24 * MIB).contains(&window), "{window}");
    // The default call (the last 20 blocks) has a floor; a limit that does not parse gets the floor, never "no limit".
    assert_eq!(limit(&["log", "--format", "json"]), 8 * MIB);
    assert_eq!(limit(&["log", "--limit", "x", "--format", "json"]), 8 * MIB);
    // A limit so large that the multiplication would overflow is clamped to the ceiling.
    assert_eq!(
        limit(&["log", "--limit", "18446744073709551615"]),
        192 * MIB
    );
    // cat: the bytes asked for plus slack; metadata is small; no --max-bytes is twice prikk's own bound.
    assert_eq!(
        limit(&["cat", "--path", "a", "--max-bytes", "1000"]),
        1000 + MIB
    );
    assert_eq!(
        limit(&["cat", "--path", "a", "--max-bytes", "26214400"]),
        26_214_400 + MIB
    );
    assert_eq!(limit(&["cat", "--path", "a", "--format", "json"]), MIB);
    assert_eq!(limit(&["cat", "--path", "a"]), 50 * MIB);
    // The creation's import report: an archive at the edge bound (100 MiB, about 130,000 refs of 171 B) fits.
    assert!(limit(&["archive", "import", "a", "--adopt", "--format", "json"]) >= 2 * 130_000 * 171);
    // stderr is small and the same for every verb.
    assert_eq!(for_args(&["log"]).stderr, STDERR);
}

#[test]
fn a_verb_without_a_row_gets_the_finite_general_limit_and_verbs_are_named_by_their_first_words() {
    assert!(!is_in_table("frobnicate"));
    assert_eq!(limit(&["frobnicate"]), GENERAL.stdout);
    const { assert!(GENERAL.stdout < u64::MAX / 2) };
    for (args, verb) in [
        (
            &["trust", "maintainer", "add", "--key-id", "k"][..],
            "trust maintainer",
        ),
        (&["archive", "import", "a"], "archive import"),
        (&["archive", "verify", "a"], "archive verify"),
        (&["sync", "accept", "a"], "sync accept"),
        (&["bundle", "export", "--ref", "r"], "bundle"),
        (&["key", "status"], "key status"),
        (&["checkout", "--patch-plan"], "checkout"),
        (&["--version"], "--version"),
        (&["log", "--format", "json"], "log"),
    ] {
        assert_eq!(verb_of(args), verb);
        assert!(rule_of(verb).is_some(), "{verb}");
    }
}

/// Lists of string literals in those sources that are not prikk arguments.
const NOT_VERBS: &[&str] = &["over-max-bytes"];

/// The verbs the drivers call, read from the sources: each literal argument list that begins a call. A new call with a verb that has
/// no row (nobody measured it) fails here instead of getting the general limit unnoticed.
#[test]
fn every_verb_the_drivers_call_has_a_measured_row() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut verbs = std::collections::BTreeSet::new();
    for file in ["cli.rs", "extract.rs"] {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        // Every `[` that opens a list of string literals whose first is a lowercase word: `&["log", ...]`, `vec!["show"`, `= [\n "trust",`.
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'[' && (i == 0 || matches!(bytes[i - 1], b'&' | b'!' | b' ' | b'(')) {
                let mut j = i + 1;
                let mut lits: Vec<String> = Vec::new();
                while lits.len() < 3 {
                    while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                        j += 1;
                    }
                    if j >= bytes.len() || bytes[j] != b'"' {
                        break;
                    }
                    let end = text[j + 1..].find('"').map(|e| j + 1 + e);
                    let Some(end) = end else { break };
                    lits.push(text[j + 1..end].to_owned());
                    j = end + 1;
                    while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b',' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if let Some(first) = lits.first()
                    && !first.is_empty()
                    && first.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                    && (!first.starts_with("--") || first == "--version")
                    && !NOT_VERBS.contains(&first.as_str())
                {
                    let args: Vec<&str> = lits.iter().map(String::as_str).collect();
                    verbs.insert(verb_of(&args));
                }
            }
            i += 1;
        }
    }
    // The scan is alive: it finds the calls (the verbs below are in the sources).
    for expected in [
        "log",
        "show",
        "tree",
        "cat",
        "verify",
        "doctor",
        "archive import",
        "archive verify",
        "sync accept",
        "sync pending",
        "extract",
        "branch",
        "tag",
        "status",
    ] {
        assert!(
            verbs.contains(expected),
            "the scan missed `{expected}`: {verbs:?}"
        );
    }
    let missing: Vec<&String> = verbs.iter().filter(|v| !is_in_table(v)).collect();
    assert!(
        missing.is_empty(),
        "verbs the drivers call without a measured row in output_cap::TABLE: {missing:?}"
    );
}
