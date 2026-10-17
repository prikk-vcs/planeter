# planeter — dependency policy and supply-chain ledger

| | |
|---|---|
| Document | The rules that produced planeter's dependency tree, and the record of every supply-chain decision (accepted with its measured cost, rejected with its reason), so settled battles are not re-fought and new ones are decided the same way. |
| As of | 2026-10-15 (lock: 200 runtime crates + 11 test-only; unchanged by the advisory fix) |
| Basis | RFC 010 (D-3 supply-chain gate at project events, D-4 triage ladder, D-5 pinned build chain); RFC 001 (the sandboxed-subprocess pattern), threat model `T-11` (supply chain) and `C-8` (egress), `deny.toml`, `.cargo/audit.toml`, the 0.1.0–0.2.0 review packages |

## Rules

1. **Measure before commit.** Any manifest change is a visible, separate step: build with the candidate,
   count the lock (`grep -c 'name = ' Cargo.lock`), run `cargo deny check` and `cargo audit`, and report
   the delta *before* the commit that adopts it, in the form "N → M crates (+K); deny: …; audit: …;
   new licenses: …". A scratch measurement never rides inside a larger command.
2. **Licenses are an allow-list** (`deny.toml`): Apache-2.0, MIT, Apache-2.0 WITH LLVM-exception,
   Unicode-3.0, MPL-2.0 (file-level copyleft; enters via ammonia's CSS parser), BSD-3-Clause (axum's
   `matchit`), ISC (dev-only `simple_asn1`). Adding one is a decision request to the owner with the
   crate, the path by which it enters, and why the license is compatible with Apache-2.0.
3. **Advisories are not ignored, except scoped and justified in both files** (`deny.toml` with a
   `reason`, `.cargo/audit.toml`). The one standing ignore: RUSTSEC-2023-0071 (Marvin, `rsa`) —
   planeter performs public-key verification only; no RSA private-key operation exists in the tree.
   An ignore is re-examined at every dependency change that touches its path.
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

`tools/floor-check` fails unless `MIN_PRIKK_VERSION`, `PRIKK_VERSION` in `release.yml` and `ARG
PRIKK_VERSION` in the `Dockerfile` are one `X.Y.Z`. `tools/pin-check` fails on any workflow `uses:` that
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
release dry run. Deliberately out of the gate's scope: the version of `cargo-deny`/`cargo-audit` installed
by the workflows (current on purpose, so they can read the advisory database) and the content of a named
runner image, which GitHub updates in place.

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

## Ledger — rejected (do not re-propose without new facts)

| Date | Candidate | Why |
|---|---|---|
| 2026-09-23 | `ssh-key` (OpenSSH key parsing) | locks the unfixable `rsa` advisory into the lock with no scoping possible → hand-rolled ed25519 parser (fingerprints identical to `ssh-keygen -lf`) |
| 2026-09-23 | `maud`, `axum-extra` (cookies) | open advisories in their trees → askama + hand-rolled cookie helpers |
| 2026-09-23 | in-process `rustls` + `russh` (TLS and SSH transport) | +165 crates; cargo-deny licenses **and** advisories failing; russh's own advisory (RUSTSEC-2026-0154) plus `rsa` → **proxy-terminated TLS + host OpenSSH `ForceCommand`** (owner-ruled) |
| 2026-09-24 | `jsonwebtoken` 9 / `ring` or `aws-lc-rs` backends | +39 crates and a cargo-deny license failure → RustCrypto backend (owner-ruled) |
| 2026-09-24 | `reqwest` / `hyper` + `rustls` (outbound HTTPS for OIDC) | reintroduces the rejected TLS tree → confined `curl` through the egress guard (owner-ruled) |

## Runtime prerequisites (not crates, but part of the supply chain)

prikk ≥ 0.52.0 (checksum-verified release asset in the image; the version pin refuses older; raised from 0.46.0 at the 0.52.0 re-baseline),
bubblewrap, curl (SSO only). Each is named in `build-info.txt`, the release notes and RELEASING.
