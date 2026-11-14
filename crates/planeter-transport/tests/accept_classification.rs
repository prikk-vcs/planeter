//! D1 of the 0.56/0.57 re-baseline: a failed accept (prikk's exit 1, not a lock conflict) is *refused* only on positive evidence that the
//! store did not change: the pending set **and** the store signature are as they were. Everything else is *incomplete*. The decision is
//! a pure function ([`refused_on_evidence`]), tested here with stand-in readers; the real-prikk tests are in `crates/planeter/tests/propose_kill.rs`
//! (a write error after the patches are stored; the refusals that change nothing).

use planeter_core::admission::StoreFile;
use planeter_transport::refused_on_evidence;

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

fn sig(files: &[(&str, u64, u128)]) -> Vec<StoreFile> {
    files
        .iter()
        .map(|(p, n, t)| ((*p).to_owned(), *n, *t))
        .collect()
}

#[test]
fn the_pending_set_and_the_signature_both_as_they_were_is_refused() {
    let before = sig(&[("containers/patch/a.container", 100, 5), ("FORMAT", 2, 1)]);
    assert!(refused_on_evidence(
        &ids(&["p1", "p2"]),
        Some(&ids(&["p2", "p1"])), // order does not matter: it is a set
        Some(&before),
        Some(&before.clone()),
    ));
    assert!(refused_on_evidence(&[], Some(&[]), Some(&[]), Some(&[])));
}

#[test]
fn a_changed_signature_with_the_pending_set_as_it_was_is_incomplete() {
    // The case that the old rule called refused: patches and blobs written, the claims not (a write error), `sync pending` unmoved.
    let pending = ids(&["p1"]);
    let before = sig(&[("containers/patch/a.container", 100, 5)]);
    for after in [
        sig(&[("containers/patch/a.container", 160, 5)]), // grew
        sig(&[("containers/patch/a.container", 100, 6)]), // rewritten with the same size
        sig(&[]),                                         // a file gone
        sig(&[
            ("containers/patch/a.container", 100, 5),
            ("containers/blob/a.container", 10, 7), // a file new
        ]),
    ] {
        assert!(
            !refused_on_evidence(&pending, Some(&pending), Some(&before), Some(&after)),
            "{after:?}"
        );
    }
}

#[test]
fn an_unreadable_signature_or_pending_set_is_incomplete() {
    let s = sig(&[("FORMAT", 2, 1)]);
    let p = ids(&["p1"]);
    // The signature before could not be read (a bound hit, a path unreadable); or the one after; or the pending set after.
    assert!(!refused_on_evidence(&p, Some(&p), None, Some(&s)));
    assert!(!refused_on_evidence(&p, Some(&p), Some(&s), None));
    assert!(!refused_on_evidence(&p, Some(&p), None, None));
    assert!(!refused_on_evidence(&p, None, Some(&s), Some(&s)));
}

#[test]
fn a_changed_pending_set_is_incomplete_whatever_the_signature_says() {
    let s = sig(&[("FORMAT", 2, 1)]);
    assert!(!refused_on_evidence(
        &ids(&["p1"]),
        Some(&ids(&["p1", "p2"])),
        Some(&s),
        Some(&s)
    ));
    assert!(!refused_on_evidence(
        &ids(&["p1"]),
        Some(&[]),
        Some(&s),
        Some(&s)
    ));
}
