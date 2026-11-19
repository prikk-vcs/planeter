//! **Unknown fields (the JSON-reliance inventory, Q2).** Every deserializer of prikk output in this crate ignores a field it does not know (the
//! serde default; no `deny_unknown_fields` is on any prikk type): a document with an extra field **at the top and in every nested object**
//! still parses, and reads the same as without it.
//!
//! The documents are the real output of prikk 0.58.0 (`tests/fixtures/real-0.58.0/`, produced by the host's prikk on a small repository of two
//! sealed blocks, a tag, a pending claim and an adopted archive; absolute paths rewritten to `/srv/r`). The extra field is added by walking the
//! parsed JSON and putting `"zz_field_prikk_adds_later": <value>` into **every object**, in every array, at every depth, with a value of each
//! JSON kind in turn (number, string, null, array, object).
//!
//! **If a prikk release changes a shape and this test fails** (a document no longer parses, or an extra field changes what is read): that is the
//! test working. Read what changed in the release's letter, change the reader in `src/model.rs`/`exchange.rs`/`extract.rs`/`delivery.rs`
//! (and its tests), then **make the documents again** with the new prikk and replace the files in `tests/fixtures/real-0.58.0/` (rename the
//! directory to the new version and the `fixture(...)` paths here). The documents are the output of these commands on a small throwaway
//! repository (two sealed blocks of a few files, a tag, a pending claim, an adopted archive; `prikk key generate --out
//! $HOME/prikk/{author,maintainer}.seed` first, the maintainer key trusted with `trust maintainer add`), with the absolute paths rewritten to
//! `/srv/r` and nothing else edited: `log --format json --ref heads/main`, `show <patch id> --format json`, `verify --format json`, `branch
//! --format json --all`, `tag --format json`, `status --format json` and `worktree-status --format json` (with a changed file), `tree --format
//! json --ref heads/main`, `cat --path <binary file> --format json --ref heads/main`, `checkout --patch-plan --format json --ref heads/main
//! --content-path <file> --content-path <binary> --content-path missing`, `key status --format json`, `trust maintainer list --format json`,
//! `trust maintainer check --key-id ed25519-0000000000000000 --format json`, `archive export F` then `archive verify F --format json`,
//! `archive import F --adopt --format json` (in a second repository that trusts the key), `extract --ref heads/main DEST --format json`,
//! `extract --ref heads/nope DEST --format json` (the refusal), `sync have`/`sync build`/`sync accept` into the second repository then `sync
//! pending --format json`, and `extract --claim <a claim id> DEST --format json`. Check the result for a path, a name or a key that is not
//! the throwaway's before it is committed.
//!
//! What this does **not** say: an unknown *value* of a known field (an enum id, a stage `status`, a content `kind`) is another question
//! (Q3); the second half of this file says which of those are closed, with the same documents.

use planeter_prikk::cli::{CliPrikkRepo, MIN_PRIKK_VERSION};
use planeter_prikk::delivery::parse_import_refusal;
use planeter_prikk::exchange::parse_pending_json;
use planeter_prikk::extract::{parse_refusal, parse_report};
use planeter_prikk::model::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}")),
    )
    .unwrap()
}

/// Put an extra field into every object of `v`, at every depth. Returns how many objects were touched.
fn add_everywhere(v: &mut Value, extra: &Value) -> usize {
    match v {
        Value::Object(map) => {
            let mut n = 1;
            for (key, child) in map.iter_mut() {
                // `written_objects` and `merged_objects` are maps from an object type to a count: a new *key* there is a new object type, which is
                // counted (see `a_new_object_type_in_an_import_report_is_counted`), not an unknown field.
                if key == "written_objects" || key == "merged_objects" {
                    continue;
                }
                n += add_everywhere(child, extra);
            }
            map.insert("zz_field_prikk_adds_later".to_owned(), extra.clone());
            n
        }
        Value::Array(items) => items.iter_mut().map(|i| add_everywhere(i, extra)).sum(),
        _ => 0,
    }
}

fn extras() -> Vec<Value> {
    vec![
        json!(1),
        json!(18446744073709551615u64),
        json!("text"),
        json!(null),
        json!([1, {"a": 2}]),
        json!({"nested": {"deeper": [1, 2]}}),
    ]
}

/// The import report's two maps are keyed by object type, and a type it has not seen is counted, not refused and not dropped: `object_count()`
/// (what the creation reports) includes it.
#[test]
fn a_new_object_type_in_an_import_report_is_counted() {
    let plain: ArchiveImportReport =
        serde_json::from_value(fixture("real-0.58.0/archive-import.json")).unwrap();
    let mut v = fixture("real-0.58.0/archive-import.json");
    v["written_objects"]["an-object-type-prikk-adds-later"] = json!(5);
    let r: ArchiveImportReport = serde_json::from_value(v).unwrap();
    assert_eq!(r.object_count(), plain.object_count() + 5);
}

/// The type parses from the document, and from the document with the extra field everywhere, to the same value.
fn tolerates<T: DeserializeOwned + PartialEq + std::fmt::Debug>(name: &str) -> usize {
    let plain = fixture(name);
    let want: T = serde_json::from_value(plain.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut objects = 0;
    for extra in extras() {
        let mut v = plain.clone();
        objects = add_everywhere(&mut v, &extra);
        let got: T = serde_json::from_value(v).unwrap_or_else(|e| {
            panic!("{name}: an extra field ({extra}) at every depth is refused: {e}")
        });
        assert_eq!(got, want, "{name}: an extra field changed what was read");
    }
    objects
}

#[test]
fn every_json_report_of_the_driver_ignores_a_field_it_does_not_know() {
    let mut total = 0;
    total += tolerates::<LogReport>("real-0.58.0/log.json");
    total += tolerates::<ShowReport>("real-0.58.0/show.json");
    total += tolerates::<VerifyReport>("real-0.58.0/verify.json");
    total += tolerates::<VerifyReport>("verify-0.54.1-unreached.json");
    total += tolerates::<BranchList>("real-0.58.0/branch.json");
    total += tolerates::<TagList>("real-0.58.0/tag.json");
    total += tolerates::<StatusReport>("real-0.58.0/status.json");
    total += tolerates::<WorktreeStatusReport>("real-0.58.0/worktree-status.json");
    total += tolerates::<PatchPlanContent>("real-0.58.0/patch-plan-content.json");
    total += tolerates::<TreeListing>("real-0.58.0/tree.json");
    total += tolerates::<PathContentMeta>("real-0.58.0/path-content.json");
    total += tolerates::<KeyStatus>("real-0.58.0/key-status.json");
    total += tolerates::<TrustList>("real-0.58.0/trust-list.json");
    total += tolerates::<TrustCheck>("real-0.58.0/trust-check.json");
    total += tolerates::<ArchiveVerifyReport>("real-0.58.0/archive-verify.json");
    total += tolerates::<ArchiveImportReport>("real-0.58.0/archive-import.json");
    // the documents were not trivial: many objects, nested
    assert!(total > 60, "{total} objects");
}

/// The readers that go through `parse_schema_json` (`schema_version` exact, then the type): through their public functions.
#[test]
fn the_extract_pending_and_refusal_readers_ignore_a_field_they_do_not_know() {
    type Case = (&'static str, fn(&str) -> bool);
    let cases: [Case; 6] = [
        ("real-0.58.0/extract-report.json", |s| {
            parse_report(s).is_ok()
        }),
        ("real-0.58.0/extract-claim.json", |s| {
            parse_report(s).is_ok()
        }),
        ("real-0.58.0/extract-refusal.json", |s| {
            parse_refusal(s).is_ok()
        }),
        ("real-0.58.0/sync-pending.json", |s| {
            parse_pending_json(s).is_ok()
        }),
        ("refusal-0.57.0-archive-import-rollback.json", |s| {
            parse_import_refusal(s).is_ok()
        }),
        ("refusal-0.57.0-bundle-import-diverged.json", |s| {
            parse_import_refusal(s).is_ok()
        }),
    ];
    for (name, parses) in cases {
        let plain = fixture(name);
        assert!(parses(&plain.to_string()), "{name} does not parse as it is");
        for extra in extras() {
            let mut v = plain.clone();
            let n = add_everywhere(&mut v, &extra);
            assert!(n >= 1);
            assert!(
                parses(&v.to_string()),
                "{name}: an extra field ({extra}) at every depth is refused"
            );
        }
    }
    // and the values read are the same
    let plain = parse_report(&fixture("real-0.58.0/extract-claim.json").to_string()).unwrap();
    let mut v = fixture("real-0.58.0/extract-claim.json");
    add_everywhere(&mut v, &json!({"x": 1}));
    assert_eq!(parse_report(&v.to_string()).unwrap(), plain);
    assert!(!plain.chain.is_empty(), "the claim fixture has a chain");
}

/// The floor the documents were produced on: a new floor means these are to be produced again.
#[test]
fn the_documents_are_those_of_the_floor_release() {
    assert!(MIN_PRIKK_VERSION >= (0, 58, 0));
    let _ = CliPrikkRepo::new("/nonexistent");
}

// ---------------------------------------------------------------------------------------------------- the other half: values

/// An unknown *value* of a closed vocabulary is an **error**, never a default: the stage `status` of a verify report, the `kind` of a content
/// union, a `schema_version` other than the expected one. (The open ones are in `unknown_values_of_open_vocabularies_are_kept`.)
#[test]
fn an_unknown_value_of_a_closed_vocabulary_is_an_error_not_a_default() {
    let mut v = fixture("real-0.58.0/verify.json");
    v["stages"][0]["status"] = json!("a_status_prikk_adds_later");
    assert!(serde_json::from_value::<VerifyReport>(v).is_err());

    let mut v = fixture("real-0.58.0/patch-plan-content.json");
    v["content"][0]["content"]["kind"] = json!("a-kind-prikk-adds-later");
    assert!(serde_json::from_value::<PatchPlanContent>(v).is_err());

    let mut v = fixture("real-0.58.0/extract-report.json");
    v["point_kind"] = json!("a-point-kind-prikk-adds-later");
    assert!(parse_report(&v.to_string()).is_err());

    let mut v = fixture("real-0.58.0/extract-report.json");
    v["schema_version"] = json!("extract-report-v2");
    assert!(parse_report(&v.to_string()).is_err());
}

/// An unknown value of an open vocabulary is kept as it came: `Other(String)`, a plain string field, never an error.
#[test]
fn unknown_values_of_open_vocabularies_are_kept() {
    // refusal ids (extract and delivery), a `reason`
    let mut v = fixture("real-0.58.0/extract-refusal.json");
    v["refusal"] = json!("a-refusal-prikk-adds-later");
    let r = parse_refusal(&v.to_string()).unwrap();
    assert_eq!(r.refusal.id(), "a-refusal-prikk-adds-later");
    v["reason"] = json!("a-reason-prikk-adds-later");
    assert!(parse_refusal(&v.to_string()).is_ok());

    let mut v = fixture("refusal-0.57.0-archive-import-rollback.json");
    v["refusal"] = json!("a-delivery-refusal-prikk-adds-later");
    let r = parse_import_refusal(&v.to_string()).unwrap();
    assert_eq!(r.refusal.id(), "a-delivery-refusal-prikk-adds-later");

    // a signature outcome
    let mut v = fixture("real-0.58.0/sync-pending.json");
    v["patches"][0]["claims"][0]["signature"]["outcome"] = json!("an-outcome-prikk-adds-later");
    assert!(parse_pending_json(&v.to_string()).is_ok());

    // an unreached object's type, a block's `kind` (a plain string)
    let mut v = fixture("verify-0.54.1-unreached.json");
    v["unreached_objects"][0]["object_type"] = json!("a-type-prikk-adds-later");
    let r: VerifyReport = serde_json::from_value(v).unwrap();
    assert_eq!(
        r.unreached_objects[0].object_type,
        UnreachedKind::Other("a-type-prikk-adds-later".to_owned())
    );
    let mut v = fixture("real-0.58.0/log.json");
    v["blocks"][0]["kind"] = json!("ABlockKindPrikkAddsLater");
    let r: LogReport = serde_json::from_value(v).unwrap();
    assert_eq!(r.blocks[0].kind, "ABlockKindPrikkAddsLater");
}

// ---------------------------------------------------------------------------------------------------- absent versus null

/// Where the two differ (the inventory's Q4): `claim_parent_block_id` of the extract report and `reason` of both refusal documents must be
/// **present** (null is an answer, absent is not one); elsewhere an `Option` field absent reads as `None` (serde's default for an `Option`).
#[test]
fn absent_and_null_are_different_where_the_reader_says_so_and_the_same_elsewhere() {
    // present-but-null is read; absent is refused
    let ok = fixture("real-0.58.0/extract-report.json");
    assert!(ok["claim_parent_block_id"].is_null());
    assert!(parse_report(&ok.to_string()).is_ok());
    let mut v = ok.clone();
    v.as_object_mut().unwrap().remove("claim_parent_block_id");
    assert!(parse_report(&v.to_string()).is_err());

    let ok = fixture("real-0.58.0/extract-refusal.json");
    assert!(ok["reason"].is_null());
    let mut v = ok.clone();
    v.as_object_mut().unwrap().remove("reason");
    assert!(parse_refusal(&v.to_string()).is_err());

    let ok = fixture("refusal-0.57.0-archive-import-rollback.json");
    let mut v = ok.clone();
    v.as_object_mut().unwrap().remove("reason");
    assert!(parse_import_refusal(&v.to_string()).is_err());

    // an `Option` without `required_nullable`: absent = null = None
    let mut v = fixture("real-0.58.0/tree.json");
    assert!(v["prefix"].is_null());
    let with_null: TreeListing = serde_json::from_value(v.clone()).unwrap();
    v.as_object_mut().unwrap().remove("prefix");
    let absent: TreeListing = serde_json::from_value(v).unwrap();
    assert_eq!(with_null, absent);

    let mut v = fixture("real-0.58.0/trust-check.json");
    v.as_object_mut().unwrap().remove("public_key");
    let c: TrustCheck = serde_json::from_value(v).unwrap();
    assert_eq!(c.public_key, None);

    // a **required** non-Option field absent is an error (the `schema_version` check apart)
    let mut v = fixture("real-0.58.0/branch.json");
    v.as_object_mut().unwrap().remove("received");
    assert!(serde_json::from_value::<BranchList>(v).is_err());
}

// ---------------------------------------------------------------------------------------------------- large integers

/// Every number of prikk JSON is read into an integer type (`u64`, or `u32` for a mode): a value above 2^53 is kept exactly, and one that does
/// not fit the type is an error, never a rounded double.
#[test]
fn numbers_are_integers_and_a_value_above_two_to_the_53_is_exact_or_refused() {
    let mut v = fixture("real-0.58.0/log.json");
    v["blocks"][0]["update_seq"] = json!(9007199254740993u64); // 2^53 + 1: not representable as a double
    let r: LogReport = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(r.blocks[0].update_seq, 9007199254740993u64);
    // through the text, as the driver reads it (a string first, then a `Value`, then the type)
    let r2: LogReport =
        serde_json::from_value(serde_json::from_str::<Value>(&v.to_string()).unwrap()).unwrap();
    assert_eq!(r2.blocks[0].update_seq, 9007199254740993u64);
    // u64::MAX is exact
    v["blocks"][0]["update_seq"] = json!(u64::MAX);
    let r: LogReport = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(r.blocks[0].update_seq, u64::MAX);
    // above u64: an error (serde_json holds it as a double, which no integer type takes)
    let text = v
        .to_string()
        .replace("18446744073709551615", "18446744073709551616");
    let over: Value = serde_json::from_str(&text).unwrap();
    assert!(serde_json::from_value::<LogReport>(over).is_err());
    // a fractional number where an integer is read: an error
    v["blocks"][0]["update_seq"] = json!(1.5);
    assert!(serde_json::from_value::<LogReport>(v).is_err());
    // a file mode is a u32: a value above it is an error
    let mut v = fixture("real-0.58.0/tree.json");
    v["entries"][0]["mode"] = json!(4294967296u64);
    assert!(serde_json::from_value::<TreeListing>(v).is_err());
}
