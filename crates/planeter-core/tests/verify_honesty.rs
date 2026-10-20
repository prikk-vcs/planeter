//! IS-39, the honesty invariant of the verify badge: the **verified** assurance appears only when prikk's
//! verdict is `ok` **and** every stage was evaluated — over every small combination of verdict and stages,
//! including the combinations prikk never sends.

use planeter_core::{Assurance, VerifyView};
use planeter_prikk::model::VerifyReport;

fn view(ok: bool, stages: &[&str]) -> VerifyView {
    let st = stages
        .iter()
        .enumerate()
        .map(|(i, s)| match *s {
            "evaluated" => format!(r#"{{"stage":"s{i}","status":"evaluated"}}"#),
            "failed" => format!(r#"{{"stage":"s{i}","status":"failed","message":"m"}}"#),
            "not_evaluated" => {
                format!(r#"{{"stage":"s{i}","status":"not_evaluated","blocked_by":"s0"}}"#)
            }
            _ => format!(r#"{{"stage":"s{i}","status":"halted","after":"s0"}}"#),
        })
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        r#"{{"schema_version":"verify-report-v1","verdict":{{"ok":{ok},"failed_conditions":[]}},"active_sessions":{{"count":1,"verified_count":1}},"stages":[{st}]}}"#
    );
    VerifyView::from_verify(&serde_json::from_str::<VerifyReport>(&json).unwrap())
}

#[test]
fn verified_only_when_ok_and_every_stage_is_evaluated() {
    const KINDS: [&str; 4] = ["evaluated", "failed", "not_evaluated", "halted"];
    let mut checked = 0;
    // every stage list of length 0..=3 over the four statuses, with both verdicts
    let mut lists: Vec<Vec<&str>> = vec![vec![]];
    for len in 1..=3 {
        let mut next = vec![];
        for base in lists.iter().filter(|l| l.len() == len - 1) {
            for k in KINDS {
                let mut l = base.clone();
                l.push(k);
                next.push(l);
            }
        }
        lists.extend(next);
    }
    for stages in &lists {
        for ok in [true, false] {
            let verified = view(ok, stages).assurance() == Assurance::PrikkVerified;
            let should = ok && stages.iter().all(|s| *s == "evaluated");
            assert_eq!(verified, should, "ok={ok} stages={stages:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, 2 * (1 + 4 + 16 + 64));
}
