//! RFC 004 part 4b: the largest accept the defaults allow, against the default memory budget of hosts of different sizes.
//! The start-up log says whether the heavy pool admits it (`planeter_web::budget::accept_admitted` is what it asks); this pins
//! which hosts those are, so that a change of the model or of a default is seen here and in the runbook together.

use planeter_transport::ProposeConfig;
use planeter_web::PrikkGate;
use planeter_web::budget::{MIB, accept_admitted, default_budget};

fn heavy_and_budget(usable_gib: u64) -> (u64, u64) {
    let (budget, _) = default_budget(usable_gib << 30);
    let gate = PrikkGate::default().with_budget(budget);
    (gate.pool_bytes().expect("a budget is set").1, budget)
}

#[test]
fn the_largest_accept_is_admitted_by_the_default_budget_from_about_eight_gib_of_memory() {
    let c = ProposeConfig::default();
    let cost = c.accept_cost(c.max_artifact_bytes);
    // 10 MiB + 7.0 x 256 MiB + 2 KiB x 5,000 = about 1.77 GiB, and the heavy pool is 7/8 of a quarter of the host.
    for (gib, admitted) in [
        (2, false),
        (4, false),
        (8, false),
        (9, true),
        (16, true),
        (64, true),
    ] {
        let (heavy, budget) = heavy_and_budget(gib);
        assert_eq!(
            accept_admitted(cost, heavy, budget),
            admitted,
            "a host of {gib} GiB: heavy pool {} MiB, the largest accept needs {} MiB",
            heavy / MIB,
            cost / MIB
        );
    }
}

#[test]
fn a_budget_of_zero_is_no_gate_and_admits_everything() {
    assert!(accept_admitted(u64::MAX, 0, 0));
    assert!(!accept_admitted(2 * MIB, MIB, 8 * MIB));
    assert!(accept_admitted(MIB, MIB, 8 * MIB));
}
