# planeter

**The forge for the [prikk](https://github.com/prikk-vcs/prikk) version-control system** — the web
service that hosts prikk repositories and the collaboration around them: repositories, review, issues,
CI, and packages.

> **Status: design phase (v0, pre-implementation).** The design set, roadmap, and the full RFC set are
> written and accepted; no forge code exists yet. Everything here is *design*; implementation follows,
> and **1.0 is an explicit owner gate.**

## Overview

planeter is to prikk what a forge like Forgejo is to Git: prikk is the local, offline version-control
system; **planeter is the hosting layer above it.** It exists so that prikk can stay a lean, offline
CLI — planeter **carries the hosting weight prikk deliberately refuses** (network, many repositories,
accounts, authorization), and builds it all on prikk's stable command surface, not its internals.

## What makes it different

- **The forge holds no history-signing key.** A push ingests *author-signed* patches keyless; a merge is
  a maintainer's *own* seal. A compromised planeter can **deny** history but can never **forge** it —
  the property a *prikk* forge exists to offer over a Git one. (Enforced construction-level: the
  forge-signing path is an off-by-default build feature, checked absent in CI.)
- **A canonical branch many maintainers advance, keyless.** The forge holds `main` by *adopting*
  trusted-maintainer-signed fast-forward advances (prikk RFC 154) — multi-maintainer, always-on, no
  forge key.
- **Built on prikk's CLI, never its unstable crate.** planeter drives prikk as a sandboxed subprocess
  over its machine-readable read surface and artifact exchange, and re-derives every repository fact
  from prikk (prikk stays the source of truth).
- **Familiar, but lean.** Repositories, clone/push, change review and merge, issues, CI, and a package
  registry — the recognizable loop — while leaning on established standards and deferring the genuine
  frontier (federation, portable identity, built-in AI).
- **Vendor-neutral groundwork.** The standards, proposals, and guidelines planeter is built on are
  documented separately in [forge-commons](https://github.com/kos-commons/forge-commons).

## Following the design

There is no build to run yet. To read the design:

- **[`ROADMAP.md`](ROADMAP.md)** — the wide perspective: milestones M0→1.0, release cycles, the v0→v1
  owner gate, and the prikk dependencies.
- **[`rfcs/README.md`](rfcs/README.md)** — the RFC pipeline (001–009, all accepted with handoffs), in
  priority order.
- **The design set** in [`docs/src/`](docs/src/):
  - `planeter-01-requirements-spec` — what the forge must do / never do / decide.
  - `planeter-02-external-design` — the black-box surfaces.
  - `planeter-03-threat-model` — security (a forge holds authority and takes untrusted network input).
  - `planeter-04-internal-design` — the white-box architecture.
  - `planeter-prikk-dependency-ledger` — every prikk behaviour planeter relies on, and how sure we are.

## Design notes

- **prikk dependencies are tracked, not assumed.** The keyless canonical-branch, merge, relay, and
  migration paths depend on prikk shipping RFC 154 (accepted) and RFC 155 (proposed); implementation is
  **gated on the prikk binary**, while the read/host/auth spine (M1) needs no prikk change. See the
  dependency ledger.
- **Layering is a gate, not a diagram.** Dependencies point downward only; authorization is one core
  service consulted on every path; only the prikk-integration layer touches prikk — each enforced in CI.

## License

Apache-2.0 (see [`LICENSE`](LICENSE)). Author: nabbisen.
