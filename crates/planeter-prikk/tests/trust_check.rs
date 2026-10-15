//! T5 (prikk 0.52.0 re-baseline, IS-21 / PK-42): `trust_check` and the exit code of "not trusted".
//!
//! prikk ≤ 0.46.0 exits 0 for "not trusted"; 0.52.0 exits 1 and still writes the `trust-check-v1`
//! document. A stand-in `prikk` script plays each shape (it needs no prikk binary), selected by the
//! `--key-id` it is given. The documents are the ones recorded from the real 0.52.0 binary on
//! 2026-10-15. The 0.46.0 shape is the **same document with exit 0**, taken from the ledger and prikk's
//! letter: no 0.46.0 binary was available to record it from.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use planeter_prikk::{CliPrikkRepo, PrikkError, PrikkRepo, Sandbox};

const SCRIPT: &str = r#"#!/bin/sh
# args: trust maintainer check --key-id <KID> --format json
KID="$5"
untrusted='{"schema_version":"trust-check-v1","key_id":"nobody-here","trusted":false,"public_key":null}'
trusted='{"schema_version":"trust-check-v1","key_id":"ed25519-7832a145102a16df","trusted":true,"public_key":"7832a145102a16dfa77f7a6eb462cc688f411f1b17f4fa314f03b2e35c6e1f4f"}'
case "$KID" in
  adopted-exit0)        echo "$trusted"; exit 0 ;;
  untrusted-exit0)      echo "$untrusted"; exit 0 ;;          # prikk <= 0.46.0 shape
  untrusted-exit1)      echo "$untrusted"; exit 1 ;;          # prikk 0.52.0 shape
  exit1-no-document)    echo "error: policy unreadable" >&2; exit 1 ;;
  exit1-garbage)        echo "not json"; exit 1 ;;
  exit1-says-trusted)   echo "$trusted"; exit 1 ;;            # contradictory: refuse
  exit1-wrong-schema)   echo '{"schema_version":"trust-check-v2","key_id":"x","trusted":false,"public_key":null}'; exit 1 ;;
  usage-exit2-document) echo "$untrusted"; exit 2 ;;          # exit 2 is never an answer
  *) echo "unexpected key id" >&2; exit 99 ;;
esac
"#;

/// One test function on purpose: the stand-in script is written once and then only executed, which
/// avoids the write-then-exec `ETXTBSY` race between parallel tests.
#[test]
fn trust_check_reads_the_answer_and_only_the_answer_from_exit_one() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("trust-check-stand-in");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("prikk");
    std::fs::write(&script, SCRIPT).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let repo = CliPrikkRepo::with_binary(&dir, &script).with_sandbox(Sandbox::Unconfined);

    // Adopted key: exit 0, trusted.
    let c = repo.trust_check("adopted-exit0").expect("adopted");
    assert!(c.trusted && c.public_key.is_some());

    // "Not trusted" is an answer in both shapes: 0.46.0 (exit 0) and 0.52.0 (exit 1).
    for kid in ["untrusted-exit0", "untrusted-exit1"] {
        let c = repo
            .trust_check(kid)
            .unwrap_or_else(|e| panic!("{kid}: {e}"));
        assert!(!c.trusted, "{kid}");
        assert_eq!(c.schema_version, "trust-check-v1");
        assert_eq!(c.public_key, None);
    }

    // Everything else non-zero stays a typed failure keyed on the exit status.
    for (kid, want) in [
        ("exit1-no-document", Some(1)),
        ("exit1-garbage", Some(1)),
        ("exit1-says-trusted", Some(1)),
        ("exit1-wrong-schema", Some(1)),
        ("usage-exit2-document", Some(2)),
        ("unexpected", Some(99)),
    ] {
        match repo.trust_check(kid) {
            Err(PrikkError::Command { code, .. }) => assert_eq!(code, want, "{kid}"),
            other => panic!("{kid}: expected a Command error, got {other:?}"),
        }
    }
}
