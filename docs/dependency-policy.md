# planeter — dependency policy and supply-chain ledger

| | |
|---|---|
| Document | The rules that produced planeter's dependency tree, and the record of every supply-chain decision (accepted with its measured cost, rejected with its reason), so settled battles are not re-fought and new ones are decided the same way. |
| As of | 2026-10-29 (lock: 189 crates; −25 for the `aws_lc_rs` backend, see the ledger; no advisory ignore) |
| Basis | RFC 010 (D-3 supply-chain gate at project events, D-4 triage ladder, D-5 pinned build chain); RFC 001 (the sandboxed-subprocess pattern), threat model `T-11` (supply chain) and `C-8` (egress), `deny.toml`, `.cargo/audit.toml`, the 0.1.0–0.2.0 review packages |

## Rules

1. **Measure before commit.** Any manifest change is a visible, separate step: build with the candidate,
   count the lock (`grep -c 'name = ' Cargo.lock`), run `cargo deny check` and `cargo audit`, and report
   the delta *before* the commit that adopts it, in the form "N → M crates (+K); deny: …; audit: …;
   new licenses: …". A scratch measurement never rides inside a larger command.
2. **Licenses are an allow-list** (`deny.toml`): Apache-2.0, MIT, Apache-2.0 WITH LLVM-exception,
   Unicode-3.0, MPL-2.0 (file-level copyleft; enters via ammonia's CSS parser), BSD-3-Clause (axum's
   `matchit`), ISC (`aws-lc-rs`, `aws-lc-sys`, `untrusted`, `simple_asn1`). Adding one is a decision request to the owner with the
   crate, the path by which it enters, and why the license is compatible with Apache-2.0.
3. **Advisories are not ignored, except scoped and justified in both files** (`deny.toml` with a
   `reason`, `.cargo/audit.toml`). **There is no standing ignore** (since 2026-10-29): the one that
   existed, RUSTSEC-2023-0071 (Marvin, `rsa`), was retired by removing the crate from the tree, not by
   arguing about reachability (see "Supply-chain triage record"). An ignore is re-examined at every
   dependency change that touches its path.
4. **Prefer a confined subprocess to an in-process client** when the capability is a whole protocol
   stack: prikk (the VCS) and curl (outbound HTTPS) run under bubblewrap with `--unshare-all --clearenv`,
   read-only system view, and only the mounts they need. An in-process TLS/SSH/HTTP stack is adopted
   only when the subprocess pattern cannot express the requirement.
5. **Keyless, unsafe-free, gated.** `#![forbid(unsafe_code)]` in every crate; no crate may bring a
   signing path for prikk history (ENF-2 checks the built artifacts); the layering gate refuses a lower
   crate depending on an upper one.
6. **`--locked` everywhere**; the lock is committed; a stale lock in a push is a bug (it happened once).

## When the gates run (RFC 010 D-3)

`cargo deny check` and `cargo audit` must exit 0 at each **project event**, not on a clock:

- **(a)** every push and pull request (`ci.yml`, job `supply-chain`);
- **(b)** every release (the `gates` job of `release.yml`);
- **(c)** every prikk re-baseline;
- **(d)** every RFC disposition checkpoint (accepted, done, superseded, withdrawn, deferred), where the
  architect runs both against a freshly fetched advisory database;
- **(e)** whenever the owner asks.

A checkpoint may leave a durable CI record with the manually dispatched `supply-chain.yml`
(`workflow_dispatch` only: no schedule, no secrets, `contents: read`). The result is one line in STATUS;
red becomes an issue-register entry. This detects an advisory at the next event, not during a quiet period.

## The build chain is pinned and checked (RFC 010 D-2, D-5)

`tools/floor-check` fails unless `MIN_PRIKK_VERSION`, `PRIKK_VERSION` in `release.yml` and in `ci.yml` (the prikk the client's tests install)
and `ARG PRIKK_VERSION` in the `Dockerfile` are one `X.Y.Z`. The base image of the `Dockerfile` is `debian:trixie-slim` by digest (D-51), and the
release workflow runs the image and checks the binary's glibc needs against it before any push (`tools/glibc-check`). `tools/pin-check` fails on any workflow `uses:` that
is not a 40-hex commit SHA, any `image:` or Dockerfile `FROM` without a digest, and any toolchain named by
channel. It also fails when the `toolchain:` inputs across the workflows are not one and the same version
(CI, the manual checkpoint and the release build with **one compiler**, `1.99.0`), and when a `runs-on` or
matrix `os` names a moving label (`*-latest`): runners are named (`ubuntu-24.04`,
`ubuntu-24.04-arm`). **The declared minimum compiler** (`rust-version` in the root `Cargo.toml`, `1.88.0`,
IS-32) means: the lowest stable compiler that builds the whole workspace with `--locked`, which is the
one that stabilized let-chains (measured: 1.85 and 1.87 fail on them, 1.88 builds and passes every test;
no dependency raises it). It is proved by the `msrv` job of `ci.yml`, which runs `cargo check --workspace
--locked` on exactly that compiler. That job is the one named exception to "one compiler": `pin-check`
fails unless its `toolchain:` equals `rust-version`, and fails if a declared minimum has no `msrv` job.
Raising the minimum is a reviewed change to `rust-version` and the job together; nothing is rewritten to
make an older compiler work. Both gates run in `ci.yml` and in the release `gates` job. **A pin is changed only by
a reviewed change that names the upstream diff**; `docs/RELEASING.md` has the pre-tag review step and the
release dry run and the rehearsal. Deliberately out of the gate's scope: the version of `cargo-deny`/`cargo-audit` installed
by the workflows (current on purpose, so they can read the advisory database) and the content of a named
runner image, which GitHub updates in place.

### The release chain trusts GitHub-owned actions only (RFC 010 D-11)

The five third-party pins that lagged upstream (IS-35: `docker/setup-qemu`, `setup-buildx`, `login`,
`build-push-action`, and `attest-build-provenance` two majors behind) and the single-maintainer
`dtolnay/rust-toolchain` were **retired, not updated**: the toolchain is installed by the runner's `rustup`,
the image is built natively per architecture with the runner's `docker`/`buildx` and a BuildKit image pinned by
digest (`moby/buildkit` v0.33.1), the registry login is `docker login --password-stdin`, and attestation is
`actions/attest` v4.2.2 (SHA `1e69f48a…`, `create-storage-record: false`). What is left outside the
repository: GitHub's `actions/checkout`, `upload-artifact`, `download-artifact`, `attest`, one BuildKit image, the
base image and the runners' own tools. `pin-check` enforces the shape: no `uses:` outside `actions/`; an
undigested `docker buildx create` image, `docker run` or `docker pull` fails; a `docker login` must read the
token with `--password-stdin` and run only on a tag push or in a rehearsal; no `secrets.*`/`github.token`
expression inside a script. Updating a remaining pin follows RFC 010 D-9 (see `docs/RELEASING.md`).

### The rehearsal cannot reach the real package (RFC 010 D-10)

`release.yml` has a `rehearsal` dispatch mode that logs in to ghcr, pushes and attests, but only to
`ghcr.io/prikk-vcs/planeter-rehearsal:rehearsal` (and, for the tarballs, attests copies named
`rehearsal-<name>`). `pin-check` makes that structural by failing when: a step naming the real image
`ghcr.io/prikk-vcs/planeter` lacks `github.event_name == 'push'` in its (or its job's) `if`; a step names both
the real image and the rehearsal package; a step naming the rehearsal package, or conditioned on
`rehearsal`, lacks `inputs.mode == 'rehearsal'` or can also run on a tag push (`github.event_name == 'push'`
or `||` in its `if`); a rehearsal step pushes anywhere but the rehearsal package; or a rehearsal step names
a `gh release`, `git tag`/`git push`, `cargo publish`/`login` or crates.io command or action. Each rule has a
planted-failure test. **Footprint of a rehearsal (owner-accepted, O-5):** one additional private package,
and attestation records for the rehearsal artifacts that live in the public transparency log permanently,
labelled `rehearsal` in their subject name. Its run id is recorded with any change to the release chain.

## Ledger — accepted

| Date | Crate(s) | Cost (runtime crates) | Notes |
|---|---|---|---|
| 2026-09-16 | `serde`, `serde_json` (driver JSON) | baseline 22 | RFC 001 |
| 2026-09-16 | `ammonia`, `pulldown-cmark`, `axum` 0.8, `tokio` | 22 → 114 | the correct-sanitizer cost (html5ever, `url`→`idna`→`icu`); owner-approved standard stack |
| 2026-09-23 | `argon2`, `sha2`, `subtle`, `base64ct` | +15 | auth crypto behind the RFC 002 seams |
| 2026-09-23 | `rusqlite` (bundled) | +16 | persistent store, one shared `SqliteDb` |
| 2026-09-23 | `askama` 0.13 | +6 | compile-time, auto-escaping templates for the browse UI |
| 2026-09-24 | `jsonwebtoken` 10 (`rust_crypto`, no default features) | 151 → 200 (+49) | OIDC verification; no `ring`/`aws-lc`; RUSTSEC-2023-0071 scoped ignore (rule 3) |
| 2026-09-24 | `jsonwebtoken` `use_pem` (dev-dependency only) | +11 test-only (lock 211) | parses the ES256 fixture key in tests; brings `simple_asn1` (ISC) |
| 2026-10-15 | `ammonia` 4.1.4 → 4.2.3, `yoke-derive` 0.8.3 → 0.8.4 (advisory fix, **lock-only**) | 211 → 211 (+0); 12 version bumps (`ammonia`, `cssparser`, `html5ever`, `markup5ever`, `phf`, `phf_codegen`, `phf_generator`, `phf_shared`, `string_cache`, `string_cache_codegen`, `web_atoms`, `yoke-derive`); deny: advisories, bans, licenses, sources ok; audit: 0 vulnerabilities; new licenses: none | RUSTSEC-2021-0157/-0158 and a yanked crate; `Cargo.toml` unchanged (`ammonia = "4"`), no ignore added; sanitizer behaviour pinned by `planeter-web/tests/sanitize_advisories.rs` on both versions |
| 2026-10-18 | `ammonia` 4.2.3 → 4.2.4 (advisory fix, **lock-only**) | 213 → 213 (+0); one version bump (`ammonia`: version and checksum lines only; no other crate moved); deny: advisories, bans, licenses, sources ok on a freshly fetched database; audit: 0 vulnerabilities (database `999ec89`, 1,338 advisories); new licenses: none | RUSTSEC-2026-0373 (`url(` matched case-sensitively under `filter_style_properties`); not reachable under `ammonia::clean` defaults; `Cargo.toml` unchanged, no ignore added; regression tests in `planeter-web/tests/sanitize_advisories.rs`. (The 211 → 213 since 2026-10-15 is the two `tools/*` workspace crates, not a dependency.) |
| 2026-10-23 | `askama` 0.13 default features trimmed (`default-features = false`, `features = ["derive", "std"]`), **manifest change** | 213 → 212 (−1: `basic-toml`; the `serde`/`serde_derive` edges only `askama_derive` and `askama_parser` held are dropped, `serde` stays for other crates); deny: advisories ok on a freshly fetched database; audit: no `basic-toml` warning | RUSTSEC-2026-0374 (`basic-toml` unmaintained, reached through `askama_derive`'s optional `config` feature, which `planeter-web` does not use: no `askama.toml`). RFC 010 D-4 rung 2, so no ignore; the comment on the dependency line says to re-enable `config` only if an `askama.toml` is needed and the advisory is resolved. Evidence: the whole test suite and the compile (no template uses a feature the trim removed) |
| 2026-10-21 | `tokio` `signal` feature in the `planeter` binary (graceful shutdown, RFC 004 part 2 T0b), **manifest change** | 212 → 214 (+2: `signal-hook-registry` 1.4.8, `errno` 0.3.14; `libc` and `windows-sys` were already there) | measured before adoption: deny 0 (advisories, bans, licenses, sources ok), audit 0 (214 crates, 1,348 advisories); both crates `MIT OR Apache-2.0`; `signal-hook-registry` is the signal registry tokio itself builds on (`vorner/signal-hook`), a 21 KB crate (48 KB of source) that uses `unsafe` internally, as installing a signal handler requires (planeter's own crates stay `forbid(unsafe_code)`); `errno` 12 KB, a handful of `unsafe` blocks around `errno` access; no `libc`-free safe alternative exists, and a hand-written one would need `unsafe` in planeter |
| 2026-10-21 | `futures-core` as a direct dependency of `planeter-web` (the `Stream` trait of the spool-file body), **manifest change** | 214 → 214 (+0: already in the lock through `axum`) | a body that reads the artifact in chunks needs the trait's name; no new crate |
| 2026-10-29 | `jsonwebtoken` 10.3.0 → 11.1.0, backend `rust_crypto` → `aws_lc_rs` (`planeter-auth`; the `planeter-web` dev-dependency too), **manifest change** (D-50) | 214 → 189 (−25: 36 crates out — `rsa`, `crypto-bigint`, `p256`, `p384`, `ecdsa`, `elliptic-curve`, `primeorder`, `ed25519-dalek` and the rest of that tree, and `jsonwebtoken` 10.3.0 itself — and 11 in: `aws-lc-rs` 1.19.0, `aws-lc-sys` 0.47.0, `untrusted`, `cmake`, `dunce`, `fs_extra`, `jobserver`, `zeroize_derive`, `getrandom` 0.4.3, `r-efi`, `jsonwebtoken` 11.1.0); deny: advisories, bans, licenses, sources ok with **no ignore** (fresh database); audit: 0 vulnerabilities (189 crates); new licenses: none needing a new allow (MIT-0, CC0-1.0 and LGPL-2.1-or-later occur only as one choice of an OR expression) | retires RUSTSEC-2023-0071 (ignore removed) and RUSTSEC-2026-0387 by removing the crates; adds a C library (`aws-lc-sys`, built with `cmake` and `cc`) to the server; binary +2 MB |

## Supply-chain triage record

| Advisory | Crate path | Reachable? How shown | Decision | Evidence |
|---|---|---|---|---|
| RUSTSEC-2023-0071 (`rsa`, Marvin timing side channel on private-key operations) | `jsonwebtoken` `rust_crypto` → `rsa` | Not by planeter's code: verification only; no RSA private key exists in the process (pinned by `crates/planeter-auth/tests/no_signing_in_production.rs`) | Was ignored (owner, 2026-09-24); **retired 2026-10-29 (D-50)** by removing the crate | `Cargo.lock` has no `rsa`; `cargo deny` and `cargo audit` exit 0 with no ignore |
| RUSTSEC-2026-0387 (`crypto-bigint` 0.5.5, timing variability on secret integers) | `jsonwebtoken` `rust_crypto` → `p256`/`p384` → `elliptic-curve` → `crypto-bigint` | Only on ES256/ES384 *verification* with public keys; no private key in the process | **Backend switched**, not ignored (D-50): the crate leaves the tree | `Cargo.lock` has no `crypto-bigint`; `cargo deny` and `cargo audit` exit 0; the 30-row algorithm matrix (`crates/planeter-auth/tests/oidc_algorithms.rs`) behaves the same on both backends |

## Ledger — rejected (do not re-propose without new facts)

| Date | Candidate | Why |
|---|---|---|
| 2026-09-23 | `ssh-key` (OpenSSH key parsing) | locks the unfixable `rsa` advisory into the lock with no scoping possible → hand-rolled ed25519 parser (fingerprints identical to `ssh-keygen -lf`) |
| 2026-09-23 | `maud`, `axum-extra` (cookies) | open advisories in their trees → askama + hand-rolled cookie helpers |
| 2026-09-23 | in-process `rustls` + `russh` (TLS and SSH transport) | +165 crates; cargo-deny licenses **and** advisories failing; russh's own advisory (RUSTSEC-2026-0154) plus `rsa` → **proxy-terminated TLS + host OpenSSH `ForceCommand`** (owner-ruled) |
| 2026-09-24 | `jsonwebtoken` 9 / `ring` or `aws-lc-rs` backends | +39 crates and a cargo-deny license failure → RustCrypto backend (owner-ruled). **History:** reversed 2026-10-29 (D-50, by the owner's delegation): the `aws_lc_rs` backend is now adopted (ledger above); the licences it needs (ISC, with BSD-3-Clause and MIT-0 in choice clauses) are permissive and were allowed |
| 2026-09-24 | `reqwest` / `hyper` + `rustls` (outbound HTTPS for OIDC) | reintroduces the rejected TLS tree → confined `curl` through the egress guard (owner-ruled) |

## Runtime prerequisites (not crates, but part of the supply chain)

prikk ≥ 0.54.0 (checksum-verified release asset in the image; the version pin refuses older; raised from 0.52.0 at the 0.54 re-baseline, 2026-10-24),
bubblewrap, curl (SSO only). Each is named in `build-info.txt`, the release notes and RELEASING.
