# planeter

**The forge for the [prikk](https://github.com/prikk-vcs/prikk) version-control system** — the web
service that hosts prikk repositories and the collaboration around them: repositories, review, issues,
CI, and packages.

> **Status: v0 — host + browse + auth are released (0.2.0); clone/push is next.** planeter hosts and
> serves prikk repositories today: multi-repository hosting, a browse UI and a read API, sign-in with
> local passwords or SSO, scoped tokens, and per-repository authorization. Clone/push waits on prikk
> shipping RFC 155 and RFC 154; review, CI and packages follow. **1.0 is an explicit owner gate.**
> Releases: [`CHANGELOG.md`](CHANGELOG.md) · plan: [`ROADMAP.md`](ROADMAP.md).

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

## Getting started

**Runtime prerequisites** (Linux only — the sandbox is bubblewrap): a `prikk` binary **≥ 0.46.0** and
`bwrap` on `PATH`, plus `curl` if you enable SSO. The container image bundles all three.

Install one of:

- **Container:** `ghcr.io/prikk-vcs/planeter:<version>` (`linux/amd64` + `linux/arm64`; persist
  `/var/lib/planeter`; bubblewrap needs user namespaces inside the container — see
  [`docs/RELEASING.md`](docs/RELEASING.md) §Deploying).
- **Release tarball:** Linux x86_64 / aarch64 builds on the
  [releases page](https://github.com/prikk-vcs/planeter/releases), each with a `.sha256` and a
  build-provenance attestation (`gh attestation verify … --repo prikk-vcs/planeter`).
- **crates.io:** `cargo install planeter --locked` — an install path, not the release artifact (it
  cannot carry the prerequisites above).

Run it:

```sh
PLANETER_ADDR=127.0.0.1:8080 PLANETER_REPOS_ROOT=./planeter-repos PLANETER_DB=./planeter.db planeter
```

planeter speaks plain HTTP and is built to sit behind a TLS-terminating reverse proxy: declare the
proxy in `PLANETER_TRUSTED_PROXIES` (a non-loopback bind is refused without it), serve raw file
downloads from a separate origin (`PLANETER_CONTENT_ORIGIN`), and set the `PLANETER_OIDC_*` variables
for SSO. Every variable is documented at the top of `crates/planeter/src/main.rs`; deployment notes
are in [`docs/RELEASING.md`](docs/RELEASING.md).

## What works today

- **Hosting** (RFC 001): repositories created and served over a bubblewrap-sandboxed prikk. The forge
  holds no signing key, and CI proves the default build has no forge-seal path.
- **Browse** (RFC 003): a web UI and a JSON read API — history, changes, directory trees, files, raw
  downloads, refs and verify status — always re-derived from prikk, rendered through a strict sanitizer
  and CSP, and honest about what prikk verified versus what the forge merely approved.
- **Auth** (RFC 002): local passwords (Argon2id), OpenID Connect SSO (accounts linked administratively),
  scoped tokens, ed25519 SSH keys, sessions with CSRF protection, per-account and per-IP login
  throttles; one pure, default-deny `authorize()` consulted on every path.
- **Not yet:** clone/push (held on prikk RFC 155/154), change review and merge, issues, CI, packages.
  The [roadmap](ROADMAP.md) has the order and the reasons.

## Following the design

The design set is the contract the code is built against:

- **[`ROADMAP.md`](ROADMAP.md)** — the wide perspective: milestones M0→1.0, release cycles, the v0→v1
  owner gate, and the prikk dependencies.
- **[`rfcs/README.md`](rfcs/README.md)** — the RFC pipeline (001–009, all accepted with handoffs;
  001–003 shipped), in priority order.
- **The design set** in [`docs/src/`](docs/src/):
  - `planeter-01-requirements-spec` — what the forge must do / never do / decide.
  - `planeter-02-external-design` — the black-box surfaces.
  - `planeter-03-threat-model` — security (a forge holds authority and takes untrusted network input).
  - `planeter-04-internal-design` — the white-box architecture.
  - `planeter-prikk-dependency-ledger` — every prikk behaviour planeter relies on, and how sure we are.

## Design notes

- **prikk dependencies are tracked, not assumed.** The keyless canonical-branch, merge, relay, and
  migration paths depend on prikk shipping RFC 155 then RFC 154 (both accepted by the prikk owner;
  scheduled after prikk 0.49.0); their implementation is **gated on the prikk binary**, while the
  read/host/auth spine (M1, shipped) needed no prikk change. See the dependency ledger.
- **Layering is a gate, not a diagram.** Dependencies point downward only; authorization is one core
  service consulted on every path; only the prikk-integration layer touches prikk — each enforced in CI.

## License

Apache-2.0 (see [`LICENSE`](LICENSE)). Author: nabbisen.
