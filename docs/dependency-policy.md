# planeter — dependency policy and supply-chain ledger

| | |
|---|---|
| Document | The rules that produced planeter's dependency tree, and the record of every supply-chain decision (accepted with its measured cost, rejected with its reason), so settled battles are not re-fought and new ones are decided the same way. |
| As of | 2026-09-24 (lock: 200 runtime crates + 11 test-only) |
| Basis | RFC 001 (the sandboxed-subprocess pattern), threat model `T-11` (supply chain) and `C-8` (egress), `deny.toml`, `.cargo/audit.toml`, the 0.1.0–0.2.0 review packages |

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

## Ledger — rejected (do not re-propose without new facts)

| Date | Candidate | Why |
|---|---|---|
| 2026-09-23 | `ssh-key` (OpenSSH key parsing) | locks the unfixable `rsa` advisory into the lock with no scoping possible → hand-rolled ed25519 parser (fingerprints identical to `ssh-keygen -lf`) |
| 2026-09-23 | `maud`, `axum-extra` (cookies) | open advisories in their trees → askama + hand-rolled cookie helpers |
| 2026-09-23 | in-process `rustls` + `russh` (TLS and SSH transport) | +165 crates; cargo-deny licenses **and** advisories failing; russh's own advisory (RUSTSEC-2026-0154) plus `rsa` → **proxy-terminated TLS + host OpenSSH `ForceCommand`** (owner-ruled) |
| 2026-09-24 | `jsonwebtoken` 9 / `ring` or `aws-lc-rs` backends | +39 crates and a cargo-deny license failure → RustCrypto backend (owner-ruled) |
| 2026-09-24 | `reqwest` / `hyper` + `rustls` (outbound HTTPS for OIDC) | reintroduces the rejected TLS tree → confined `curl` through the egress guard (owner-ruled) |

## Runtime prerequisites (not crates, but part of the supply chain)

prikk ≥ 0.46.0 (checksum-verified release asset in the image; the version pin refuses older),
bubblewrap, curl (SSO only). Each is named in `build-info.txt`, the release notes and RELEASING.
