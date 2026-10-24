//! RFC 004 parts 4b and 4c: the largest accept the defaults allow, against the default memory budget of hosts of different sizes.
//! The start-up log says whether the heavy pool admits it (`planeter_web::budget::accept_startup_line` is the sentence, built on
//! `accept_admitted`); this pins which hosts those are, so that a change of the model or of a default is seen here and in the runbook.

use planeter_transport::ProposeConfig;
use planeter_web::PrikkGate;
use planeter_web::budget::{MIB, accept_admitted, accept_startup_line, default_budget};

fn heavy_and_budget(usable_gib: u64) -> (u64, u64) {
    let (budget, _) = default_budget(usable_gib << 30);
    let gate = PrikkGate::default().with_budget(budget);
    (gate.pool_bytes().expect("a budget is set").1, budget)
}

#[test]
fn the_default_bound_is_a_hundred_mib_and_a_four_gib_host_admits_it_and_a_two_gib_host_does_not() {
    let c = ProposeConfig::default();
    assert_eq!(c.max_artifact_bytes, 100 * MIB);
    let cost = c.accept_cost(c.max_artifact_bytes);
    // 10 MiB + 7.0 x 100 MiB + 2 KiB x 5,000 = about 720 MiB; the heavy pool is 7/8 of a quarter of the host.
    for (gib, admitted) in [
        (2, false),
        (3, false),
        (4, true),
        (8, true),
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
fn a_raised_bound_of_256_mib_is_admitted_from_nine_gib_and_the_line_says_so_before_that() {
    let raised = ProposeConfig {
        max_artifact_bytes: 256 * MIB,
        ..ProposeConfig::default()
    };
    let cost = raised.accept_cost(raised.max_artifact_bytes);
    assert!(cost > 1700 * MIB && cost < 1900 * MIB, "{} MiB", cost / MIB);
    for (gib, admitted) in [(4, false), (8, false), (9, true), (16, true), (64, true)] {
        let (heavy, budget) = heavy_and_budget(gib);
        assert_eq!(accept_admitted(cost, heavy, budget), admitted, "{gib} GiB");
        let (warn, line) =
            accept_startup_line(raised.max_artifact_bytes, raised.max_objects, heavy, budget);
        assert_eq!(warn, !admitted, "{gib} GiB: {line}");
    }
}

#[test]
fn the_startup_line_names_what_the_budget_admits_and_the_bound_for_a_four_and_a_two_gib_host() {
    let c = ProposeConfig::default();
    let (heavy, budget) = heavy_and_budget(4);
    let (warn, line) = accept_startup_line(c.max_artifact_bytes, c.max_objects, heavy, budget);
    assert!(!warn);
    assert_eq!(
        line,
        "the memory budget (heavy pool 896 MiB) admits an accept of an artifact up to 125 MiB; PRIKK_EXCHANGE_MAX_BYTES is 100 MiB, which fits"
    );
    let (heavy, budget) = heavy_and_budget(2);
    let (warn, line) = accept_startup_line(c.max_artifact_bytes, c.max_objects, heavy, budget);
    assert!(warn);
    assert_eq!(
        line,
        "WARNING: the memory budget (heavy pool 448 MiB) admits an accept of an artifact up to 61 MiB, but PRIKK_EXCHANGE_MAX_BYTES is 100 MiB, so a proposal larger than that is refused as too large; raise PLANETER_PRIKK_MEMORY_BUDGET_MB or lower PRIKK_EXCHANGE_MAX_BYTES to 61 MiB"
    );
    // A pool too small for even the fixed part says so instead of a made-up number.
    let (warn, line) = accept_startup_line(c.max_artifact_bytes, c.max_objects, 4 * MIB, 8 * MIB);
    assert!(warn && line.contains("admits no accept at all"), "{line}");
    // No budget, no gate.
    let (warn, line) = accept_startup_line(c.max_artifact_bytes, c.max_objects, 0, 0);
    assert!(!warn && line.contains("budget is off"), "{line}");
}

#[test]
fn a_budget_of_zero_is_no_gate_and_admits_everything() {
    assert!(accept_admitted(u64::MAX, 0, 0));
    assert!(!accept_admitted(2 * MIB, MIB, 8 * MIB));
    assert!(accept_admitted(MIB, MIB, 8 * MIB));
}
