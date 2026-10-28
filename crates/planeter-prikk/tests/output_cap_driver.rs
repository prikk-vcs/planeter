//! IS-55: what the driver does with a prikk call that prints more than its verb's limit. A stand-in `prikk` script plays the calls
//! (it needs no prikk binary). The point of the cases is the prohibited shortcut: **a truncated report is never parsed as a whole
//! one**, and `OutputTooLarge` is never `Ok`, never an empty answer, never a parse of the first bytes.
//!
//! One test function on purpose: the script is written once and then only executed (no write-then-exec `ETXTBSY` race).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use planeter_prikk::{
    CliPrikkRepo, OutputStream, PrikkError, PrikkRepo, Sandbox, output_too_large_total,
};

const MIB: u64 = 1 << 20;

/// `trust maintainer check --key-id KID --format json`: `$5` is KID. The document is valid `trust-check-v1`; the cases pad it with
/// spaces (still valid JSON: a reader that cut the output at the limit would parse it **whole**) or flood a stream.
const SCRIPT: &str = r#"#!/bin/sh
case "$1" in
  log)
    # 20 MiB of junk: over the default limit of a log, under the cap of `--limit 100000`.
    head -c 20971520 /dev/zero | tr '\0' x
    exit 0 ;;
  trust) ;;
  *) echo "unexpected verb" >&2; exit 99 ;;
esac
KID="$5"
doc='{"schema_version":"trust-check-v1","key_id":"ed25519-7832a145102a16df","trusted":true,"public_key":"7832a145102a16dfa77f7a6eb462cc688f411f1b17f4fa314f03b2e35c6e1f4f"}'
pad() { head -c "$1" /dev/zero | tr '\0' ' '; }
case "$KID" in
  exact)      printf '%s' "$doc"; pad $((1048576 - ${#doc})); exit 0 ;;
  one-more)   printf '%s' "$doc"; pad $((1048576 - ${#doc} + 1)); exit 0 ;;
  gigabyte)   printf '%s' "$doc"; head -c 1073741824 /dev/zero | tr '\0' ' '; exit 0 ;;
  stderr)     printf '%s' "$doc"; head -c 2097152 /dev/zero | tr '\0' e >&2; exit 0 ;;
  stderr-exit1) head -c 2097152 /dev/zero | tr '\0' e >&2; exit 1 ;;
  *) echo "unexpected key id" >&2; exit 99 ;;
esac
"#;

#[test]
fn a_call_over_its_limit_is_a_named_error_never_a_whole_report() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("output-cap-driver-stand-in");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("prikk");
    std::fs::write(&script, SCRIPT).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = CliPrikkRepo::with_binary(&dir, &script).with_sandbox(Sandbox::Unconfined);

    // Exactly the limit (1 MiB for `trust maintainer`): collected whole, parsed.
    let ok = repo.trust_check("exact").expect("exactly the limit");
    assert!(ok.trusted);

    // One byte more: the document is complete JSON in the first 1 MiB, so a silent truncation would parse it as an answer.
    for key in ["one-more", "gigabyte", "stderr"] {
        let before = output_too_large_total()
            .get("trust maintainer")
            .copied()
            .unwrap_or(0);
        match repo.trust_check(key) {
            Err(PrikkError::OutputTooLarge {
                verb,
                limit,
                stream,
            }) => {
                assert_eq!(verb, "trust maintainer", "{key}");
                assert_eq!(limit, MIB, "{key}");
                assert_eq!(
                    stream,
                    if key == "stderr" {
                        OutputStream::Stderr
                    } else {
                        OutputStream::Stdout
                    },
                    "{key}"
                );
            }
            other => panic!("{key}: expected OutputTooLarge, got {other:?}"),
        }
        let after = output_too_large_total()["trust maintainer"];
        assert_eq!(after, before + 1, "{key}: the counter counts the call");
    }

    // A failing call that floods stderr is the same error (not the plain failure that would carry the text).
    assert!(matches!(
        repo.trust_check("stderr-exit1"),
        Err(PrikkError::OutputTooLarge {
            stream: OutputStream::Stderr,
            ..
        })
    ));

    // The cap of a `log` grows with `--limit`: 20 MiB of junk is over a default call's limit (8 MiB) and inside `--limit 100000`'s,
    // where it is read whole and refused as garbage (Parse), which is the proof that it was collected.
    match repo.log(None, None) {
        Err(PrikkError::OutputTooLarge { verb, limit, .. }) => {
            assert_eq!(verb, "log");
            assert_eq!(limit, 8 * MIB);
        }
        other => panic!("log, default limit: expected OutputTooLarge, got {other:?}"),
    }
    assert!(
        matches!(repo.log(None, Some(100_000)), Err(PrikkError::Parse(_))),
        "log --limit 100000 admits 20 MiB"
    );
    assert_eq!(output_too_large_total()["log"], 1);
}
