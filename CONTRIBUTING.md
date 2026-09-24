# Contributing

planeter is developed by a small team under a documented workflow: a human **owner** who decides
direction, priorities, releases and the owner-only questions; an **architect** (a high-capability AI
agent) who writes requirements, designs, RFCs and handoffs and reviews every change; and a **dev team**
(a mid-capability AI agent) that implements against handoffs and returns evidence. Outside
contributions are welcome through the same door: an issue or an RFC first, code second.

## How work is reviewed here

1. **Design before implementation.** Requirements → external design → threat model → internal design →
   RFC → handoff → implementation → tests → review. Important decisions are not made implicitly in a
   patch. The design set is in [`docs/src/`](docs/src/); the RFCs in [`rfcs/`](rfcs/) with their
   lifecycle policy; the roadmap in [`ROADMAP.md`](ROADMAP.md); where things stand right now in
   [`docs/STATUS.md`](docs/STATUS.md).
2. **A change is implemented from a handoff** ([`rfcs/handoffs/README.md`](rfcs/handoffs/README.md)):
   it names the governing RFC, the change scope, the non-change scope, the required tests and the
   evidence expected back. A handoff whose status is not *assignable* is not implemented from.
3. **A review request carries evidence, not claims**: commits, every gate's command with its exit code,
   the test run with prikk on `PATH` and the run with it shimmed off (CI has no prikk), the
   dependency-tree delta when a manifest changed, deviations from the handoff, known limitations.
4. **The architect reviews** for requirements and design conformance, correctness, security, and
   release readiness, and answers *Approved* / *Corrections Required* / *Design Reconsideration* /
   *Owner Decision Required*. Passing tests alone do not approve a change.
5. **Tags, publishes, releases and letters to the prikk team are owner-authorized**, each time.

## Before you start on anything larger

Read [`docs/STATUS.md`](docs/STATUS.md) (the hold, the pending items, the issue register) and the
RFC that governs the area. A change to a public API, a persistent format, a security boundary, a
module responsibility, a dependency or the release scope is a design question: open it as an RFC or a
decision request, not as a patch.

## Building and testing

Runtime prerequisites on the development host (Linux only — the sandbox is bubblewrap):
`prikk` at the pinned floor (`MIN_PRIKK_VERSION` in `crates/planeter-prikk/src/cli.rs`; install with
`cargo install prikk --version <floor> --locked`), `bwrap`, and `curl` for SSO.

The gates, exactly as CI runs them (plus the rustdoc check):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked            # includes the layering gate; prikk-dependent tests self-skip without prikk
cargo deny check
cargo audit
tools/enf2-symbol-check/check.sh           # no forge-signer symbol in the default build
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

Run the test suite twice before pushing: once with prikk on `PATH`, once with it shimmed off (a
directory containing only `cargo`, `rustc`, `rustup` symlinks, put first on `PATH`). The second run is
what CI sees. Any manifest change follows [`docs/dependency-policy.md`](docs/dependency-policy.md):
measure, report the delta with `cargo deny` and `cargo audit` results, then commit.

Commits: one summary sentence, an explanatory body, `--locked` everywhere, the lock file committed, no
attribution trailers. Releases follow [`docs/RELEASING.md`](docs/RELEASING.md).

## Repository layout

- `crates/planeter-prikk` — the only crate that touches prikk (sandboxed subprocess, typed JSON).
- `crates/planeter-store` — records and their persistence (in-memory, SQLite).
- `crates/planeter-core` — hosting, the single default-deny `authorize()`, the honest read path, the
  egress guard and the outbound fetcher.
- `crates/planeter-auth` — accounts, sessions, throttles, OpenID Connect, tokens, SSH keys, hashing.
- `crates/planeter-web` — the browse UI and the read API, sanitization, CSP, trusted proxies.
- `crates/planeter` — the server binary. `planeter-transport`, `-ci`, `-registry`, `-runner` are
  skeletons for later milestones.
- `tools/` — the layering gate and the ENF-2 symbol check. `docs/` — design set, status, release,
  upstream and dependency records. `rfcs/` — RFCs and handoffs.

Layering is a gate: dependencies point downward only (store ← core ← auth ← web); only
`planeter-prikk` runs prikk.

## Reporting a security vulnerability

See [`SECURITY.md`](SECURITY.md). Do not open a public issue for a vulnerability.

## What this file deliberately does not have

A style guide (`rustfmt` and `clippy -D warnings` decide), a code of conduct beyond ordinary
professional courtesy, and a contributor licence agreement: contributions are accepted under the
repository's Apache-2.0 licence.
