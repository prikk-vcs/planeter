# planeter

**The forge for the [prikk](https://github.com/prikk-vcs/prikk) version-control system** — the web
service that hosts prikk repositories and the collaboration around them: repositories, review, issues,
CI, and packages.

> **Status: v0 — 0.10.0 builds on 0.5.0, the first release in which a person can propose work, have it reviewed, and create a
> repository; the forge still holds no key.** planeter hosts and serves prikk repositories (multi-repository
> hosting, a browse UI and a read API, sign-in with local passwords or SSO, scoped tokens, per-repository
> authorization), and lets a signed-in person **propose** a change (held for review; no branch moves),
> **review** it (comments and decisions), and **create** a repository (empty, or from a maintainer's archive, trusting
> only the keys the person types). `planeter-client` is the separate program for a person's own computer.
> 0.8.0 adds **landing by delivery** (RFC 008): a maintainer seals work with their own key and runs `planeter-client deliver`; the server checks the repository
> against a written rule, says what would change, and — only when the maintainer confirms — makes it the project's branch (`heads/main`). The forge signs nothing.
> Proposals whose work is in that branch show as **landed**, and their changes can be looked at. It **requires prikk 0.58.0** (a breaking prerequisite; the 0.7.0 file-by-file view
> is unchanged).
> 0.9.0 adds **maintainer keys you can see and, as the person who runs the server, add** (RFC 013): every repository page lists the keys the repository trusts and how each came,
> `planeter operator key add` adds one with the server stopped, and the creation page says how many of the 64 key slots it leaves. **No key can be removed, retired or revoked yet,
> and a delivery cannot add one.** It still **requires prikk 0.58.0**. The 37 Japanese messages of this release were read and accepted by the owner on 2026-11-25, and none is flagged.
> 0.10.0 adds **accounts the person who runs the server can make and read** (RFC 014, step 1): `planeter operator account …` and `planeter operator token …` run **beside the
> running server**, every act is written to the account's own log in sentences its holder is meant to read, disabling an account refuses its sign-ins and tokens at the next request and
> deletes nothing, and the sign-in provider (OpenID Connect) has the settings a deployment needs: a provider on a private address, how long a sign-in lasts, and **turning password
> sign-in off**. It also fixes two availability faults in the running service (IS-74, IS-75). **There are no pages yet where a person makes their own token or changes their own password**
> (step 2). planeter's sign-in code was **tested with sui-id 0.92.0 on 2026-11-27**, in process, not through a running planeter server: the limits are in [`docs/accounts.md`](docs/accounts.md).
> It still **requires prikk 0.58.0**. **160 Japanese messages of this release have not been read by the owner and ship flagged**; English is complete.
> **1.0 is an explicit owner gate.**
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

**Runtime prerequisites** (Linux only — the sandbox is bubblewrap): a `prikk` binary **≥ 0.58.0** and
`bwrap` and `prlimit` (util-linux) on `PATH`, plus `curl` if you enable SSO. planeter refuses to start
without `prlimit`. The container image bundles all of them.

Reads are admitted by estimated memory: set `PLANETER_PRIKK_MEMORY_BUDGET_MB` for the host (default a
quarter of its memory, between 256 MiB and 8 GiB; 7/8 of it — the heavy pool, beside a small light pool for
cheap reads — should be at least 4.1 × the largest repository you will `verify`). A read that cannot fit
its pool is answered `422 too_large`; see
[`docs/RELEASING.md`](docs/RELEASING.md).

Install one of:

- **Container:** `ghcr.io/prikk-vcs/planeter:<version>` (`linux/amd64` + `linux/arm64`; persist
  `/var/lib/planeter`; bubblewrap needs user namespaces inside the container — see
  [`docs/RELEASING.md`](docs/RELEASING.md) §Deploying).
- **Release tarball:** Linux x86_64 / aarch64 builds on the
  [releases page](https://github.com/prikk-vcs/planeter/releases), each with a `.sha256` and a
  build-provenance attestation (`gh attestation verify … --repo prikk-vcs/planeter`).
- **crates.io:** `cargo install planeter --locked` — an install path, not the release artifact (it
  cannot carry the prerequisites above).

**On your own computer** (a person with their own prikk and keys): `cargo install planeter-client --locked`, or build it from source
(`cargo build -p planeter-client --release --locked`). 0.5.0, 0.6.0, 0.7.0, 0.8.0, 0.9.0 and 0.10.0 ship the client through crates.io and as source; release-asset binaries of it
are not part of this release. See [`docs/client.md`](docs/client.md).

Run it:

```sh
PLANETER_ADDR=127.0.0.1:8080 PLANETER_REPOS_ROOT=./planeter-repos PLANETER_DB=./planeter.db planeter
```

planeter speaks plain HTTP and is built to sit behind a TLS-terminating reverse proxy: declare the
proxy in `PLANETER_TRUSTED_PROXIES` (a non-loopback bind is refused without it), serve raw file
downloads from a different host (`PLANETER_CONTENT_ORIGIN`; a non-loopback planeter refuses to start
without one), and set the `PLANETER_OIDC_*` variables for SSO. `planeter --help` lists every variable;
deployment notes are in [`docs/RELEASING.md`](docs/RELEASING.md).

## What works today

- **Hosting** (RFC 001): repositories created and served over a bubblewrap-sandboxed prikk. The forge
  holds no signing key, and CI proves the default build has no forge-seal path.
- **Browse** (RFC 003): a web UI and a JSON read API — history, changes, directory trees, files, raw
  downloads, refs and verify status — always re-derived from prikk, rendered through a strict sanitizer
  and CSP, and honest about what prikk verified versus what the forge merely approved.
- **Auth** (RFC 002): local passwords (Argon2id), OpenID Connect SSO (accounts linked administratively),
  scoped tokens, ed25519 SSH keys, sessions with CSRF protection, per-account and per-IP login
  throttles; one pure, default-deny `authorize()` consulted on every path.
- **Propose, review, create** (RFC 004, RFC 005): a signed-in person who may propose sends work built by their own prikk
  (`prikk sync build`); the server checks and accepts it as a **proposal** — held for review, **nothing sealed, merged or published**.
  Reviewers comment and record a decision per revision (approve or request changes); nothing here merges or blocks anything. A person can
  create a repository through the pages or the API, empty or from a maintainer's archive. A repository left unfinished by a write is
  recovered by the server when every witness agrees, or by an administrator by a written, recorded procedure.
- **Clone, fetch, propose from your computer** (RFC 004): `planeter-client` ([`docs/client.md`](docs/client.md)) — separate from the
  server, holds no key and signs nothing, https only; **only the maintainer keys you type are trusted**.
- **The file-by-file view** (RFC 005 part 6): each revision of a proposal links to a page of the files it changes and a page of each file's diff (and the same as JSON under `…/revisions/{seq}/changes`), compared with the revision's own base and rebuilt by prikk from the patches; names and text are shown display-safe, a view is bounded in time, memory and size, and nothing in it is signed ([`docs/proposals.md`](docs/proposals.md)).
- **Landing by delivery** (RFC 008, [`docs/deliveries.md`](docs/deliveries.md)): `planeter-client deliver` sends a maintainer's own sealed repository to be checked; if it
  continues the project's branch and is signed only by keys the repository already trusts, the report says what would change (the open proposals that stay, those that
  land, those that cannot be kept), and **after the maintainer confirms** it becomes `heads/main`, with the open proposals made again from the artifacts the server kept. A refusal changes nothing and
  says what to do. The repository's pages say who delivered; a landed proposal's changes are shown from the sealed history. A fresh clone has the delivered branch exactly.
- **Maintainer keys** (RFC 013, [`docs/keys.md`](docs/keys.md)): the repository and history pages (and `trusted_keys` in the `refs` JSON) list the keys the repository trusts and how each came (creation, the server's operator, a delivery, or not recorded); with the server stopped, the operator adds one with `planeter operator key add`, with a reason everyone who can see the repository reads. A key is never removed (prikk cannot do it without breaking `verify`); at most 64 keys, and the creation page says how many slots are left.
- **Accounts and tokens** (RFC 014 step 1, [`docs/accounts.md`](docs/accounts.md)): the person who runs the server makes accounts and their access tokens with `planeter operator account …` and `planeter operator token …` (beside the running server; a password is never an argument, a token is shown once and always ends, disabling an account refuses its sign-ins and tokens at once and nothing is deleted); a sign-in provider (OpenID Connect) is linked by `(issuer, subject)` only, never by email; `PLANETER_SESSION_HOURS` sets how long a sign-in lasts; `PLANETER_LOCAL_PASSWORDS=off` leaves the provider as the only way in. The page says what the provider told us and what was measured (sui-id 0.92.0, with its limits), what it risks to allow a provider on a private address, and that a person disabled at the provider must be disabled here too.
- **Languages** (RFC 012): English and Japanese (signed off by the owner on 2026-10-31, and read again on 2026-11-14, on 2026-11-17, on 2026-11-21 and on 2026-11-25, the last time with the 37 messages of the maintainer keys; **the 160 messages of 0.10.0's accounts and sign-in are not yet read and ship flagged**) — the pages, the API's error messages and `planeter-client`; a caller who does not ask is English.
- **Not yet:** removing, retiring or revoking a maintainer key, adding one by a delivery, self-service pages for accounts (a person's own tokens, password, sign-ins and security log: RFC 014 step 2), provisioning and a collaborators page (step 3), a second factor (the provider's job) or email recovery (never in planeter), a private certificate authority for the provider (and so a sign-in through a running server against one that has been measured), a periodic check that a person still exists at the provider, required approvals, several followed branches, a delta delivery (the whole history is sent each time, up to
  `PLANETER_IMPORT_MAX_MB`), a web form for delivering (a browser cannot sign), issues, checks and CI, packages. SSH is not planned: transport is https behind your
  reverse proxy. The [roadmap](ROADMAP.md) has the order and the reasons.

## Following the design

The design set is the contract the code is built against ([`CONTRIBUTING.md`](CONTRIBUTING.md) says how
work moves through it; [`SECURITY.md`](SECURITY.md) how to report a vulnerability):

- **[`docs/STATUS.md`](docs/STATUS.md)** — where the project is right now: the hold, what is pending on
  whom, the issue register. Read first.
- **[`ROADMAP.md`](ROADMAP.md)** — the wide perspective: milestones M0→1.0, release cycles, the v0→v1
  owner gate, and the prikk dependencies.
- **[`rfcs/README.md`](rfcs/README.md)** — the RFC pipeline (001–009, all accepted with handoffs;
  001–003 shipped), in priority order.
- **[`docs/SCHEDULE.md`](docs/SCHEDULE.md)** — the plan beneath the roadmap, without dates: themes, plans in
  order, and the issues, risks and concerns with their relations.
- **The design set** in [`docs/src/`](docs/src/):
  - `planeter-00-project-charter` — background, goals, what "done" means, constraints, who decides what.
  - `planeter-01-requirements-spec` — what the forge must do / never do / decide (v0.2).
  - `planeter-02-external-design` — the black-box surfaces (v0.2).
  - `planeter-03-threat-model` — security (a forge holds authority and takes untrusted network input) (v0.3).
  - `planeter-04-internal-design` — the white-box architecture (v0.2).
  - `planeter-prikk-dependency-ledger` — every prikk behaviour planeter relies on, and how sure we are.

## Design notes

- **prikk dependencies are tracked, not assumed.** The keyless canonical-branch, merge, relay, and
  migration paths depend on prikk's RFC 154 (accepted by the prikk owner, not yet scheduled; RFC 155 has
  shipped as `prikk archive` in 0.51.0); their implementation is **gated on the prikk binary**, while the
  read/host/auth spine (M1, shipped) needed no prikk change. See the dependency ledger.
- **Layering is a gate, not a diagram.** Dependencies point downward only; authorization is one core
  service consulted on every path; only the prikk-integration layer touches prikk — each enforced in CI.

## License

Apache-2.0 (see [`LICENSE`](LICENSE)). Author: nabbisen.
