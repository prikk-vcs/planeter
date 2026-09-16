# T7 — `stikk-prikk` reuse evaluation (RFC 001 D-7 / IQ-1 / OQ-7)

**Decision: keep planeter's own independent driver (`planeter-prikk`), built against the `PrikkRepo`
trait. Do not depend on `stikk-prikk`.** Reuse stikk's *lessons*, not its *code*. Revisit only if a
third prikk front-end appears (see "Escape hatch").

This records the evaluation RFC 001 D-7 required: build T3 on stikk's existing `stikk-prikk` layer
versus an independent driver, judged against the `PrikkRepo` trait. Either way callers are unaffected
(they see only `PrikkRepo`), so this is an implementation choice, not an interface one.

## What `stikk-prikk` is

The only crate in stikk that talks to prikk (stikk design `stikk-04` AR-02/MOD-02). It exposes a
`Prikk` trait with a `CliBackend` (drives the `prikk` binary) and a `NullBackend`, a version/handshake
probe (`Version`, `Handshake`, a *validated ceiling*), an `env` module that reads signing-key
**presence only, never values**, an EPIPE guard, and JSON parsing. It is **published on crates.io**
(stikk 0.1.0) and its read surface returns **`stikk-model`** types (`Orientation`, `WorktreeStatus`,
`ChangeToken`), shaped for a TUI reader that degrades to read-only on version skew.

It is genuinely good code, and the properties it localizes (presence-only key reads, EPIPE handling,
version-skew discipline, "one seam to prikk") are exactly the ones planeter also wants. The question is
whether planeter should *import* it or *re-derive* it.

## Why an independent driver

1. **The domain surfaces differ.** stikk is a read/orientation front-end; its `Prikk` trait has **no
   artifact-exchange verbs**. planeter's `PrikkRepo` needs the full `bundle_export/import` and
   `sync_summary/have/build/accept/pending/seal` set (RFC 004 transport) — the heart of a forge and
   absent from stikk. Reuse would mean extending stikk's crate with hosting concerns it has no reason
   to carry, on stikk's release schedule.

2. **The return types are the wrong shape and would couple two peers.** stikk's reads return
   `stikk-model` types. Depending on `stikk-prikk` drags `stikk-model` into planeter's boundary and
   couples **two peer front-ends** (planeter : prikk :: stikk : prikk — siblings, not layers). It also
   forces **lockstep prikk rebaselines**: stikk rebaselines on nearly every prikk release (handoffs
   009/015/017/021/029 …); planeter would inherit that cadence instead of pinning its own floor
   (prikk ≥ 0.43.0, PK-22). planeter's models are deliberately faithful to prikk's *own* JSON
   (`schema_version`-first), not to stikk's reader view.

3. **Confinement has no analogue in stikk.** planeter parses **untrusted pushed bytes**, so it wraps
   every prikk invocation in a bubblewrap sandbox — fs-scoped, no network, wall-time bounded, `--clearenv`
   (INV-3 / C-4c / T4). stikk is a local interactive tool operating on the user's *own* repository and
   runs prikk directly; it has no sandbox to reuse. This is planeter's single largest boundary concern
   and it is planeter-specific.

4. **The keyless-forge property is stronger here.** stikk reads key *presence*; planeter must hold **no
   keys at all** (INV-2). The sandbox withholds `HOME` entirely, so operator-key queries *cannot* run
   confined — a property planeter asserts by construction, not one stikk's `env` module expresses.

5. **Coupling to a published crate.** `stikk-prikk` is a crates.io library on stikk's SemVer cadence;
   planeter's crates are all `publish = false`. Taking a dependency on it would bind planeter's build to
   stikk's public API stability and version-skew policy for no offsetting gain.

## What we reuse anyway (lessons, not code)

`planeter-prikk` independently applies the patterns stikk proved: one seam to prikk (`LAY-3`), a version
pin that **refuses** out-of-range rather than guessing (D-3), typed exit-code mapping (never panic),
`schema_version` checks on every JSON read (PK-18 drift guard), and presence-not-values key handling
(the sandbox makes it absolute). The `PrikkRepo` trait keeps the implementation swappable exactly as
OQ-7 asked — so this decision costs nothing at the interface.

## Escape hatch

The one thing worth sharing is the *purely mechanical* substrate: `prikk --version` parsing, the
exit-code → error mapping, and generic `--format json` envelope handling. If a **third** consumer of
prikk's CLI ever appears, factor those into a small, vendor-neutral `prikk-cli-common` crate that
**neither stikk nor planeter owns** and both depend on — rather than making one peer depend on the
other. Two consumers do not yet justify the coordination cost; re-derivation is cheaper than the
coupling. This is recorded so the option isn't lost.

## Verdict

Independent driver, kept. No code dependency on stikk. The `PrikkRepo` trait preserves the freedom to
change this later without touching any caller.
