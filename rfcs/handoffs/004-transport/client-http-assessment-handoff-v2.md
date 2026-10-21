# RFC 004 — Transport — Handoff (v2, part 5a): assess the HTTP and TLS stack of the client helper (no product code)

| | |
|---|---|
| Document | RFC 004 D-10 calls for a client helper that talks HTTPS to the forge (advertise, conditional and ranged fetch, a framed streamed upload). The workspace has **no TLS stack and no HTTP client** (TLS ends at the operator's proxy). A new network-facing dependency set is the project's most security-sensitive addition so far, so this handoff **measures and reports; it adds nothing to the workspace**. The architect decides from the report, then writes the helper's handoff. |
| Status | **Assigned** (2026-10-22). Independent of part 4a; may run beside it. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-10, D-11, Q-2; the dependency policy (a new dependency needs a justification; `unsafe` only inside reviewed crates; `forbid(unsafe_code)` in ours); `deny.toml`; RFC 010 (MSRV 1.88.0, signed releases, the container build); the propose framing in `docs/propose.md`. |
| Audience | Dev team. Return a report as a review-request package. |

## What the client needs from the stack (the requirements to test against)

HTTPS to a forge behind an ordinary public-CA certificate and to one with a private CA given by the user; HTTP/1.1 is enough. `GET` with
`If-None-Match` (ETag) and `Range`, streaming a response of up to hundreds of MiB **to a file** without holding it in memory; `POST` of a body of
known length (`Content-Length` is **required** by the forge) streamed from a prefix plus a file; bearer-token header; timeouts (connect,
idle, total); redirects refused or limited to the same origin; proxy variables honoured (`HTTPS_PROXY`, `NO_PROXY`); a User-Agent with the
client name and version; no cookies; the ability to read an error body of the `{code, message, hint}` shape on 4xx/5xx. The build must work on
Linux, macOS and Windows with only a Rust toolchain at MSRV 1.88.0 (a C compiler or cmake is a cost to be counted, not a blocker).

## Scope: a table and a recommendation, from real builds

In a **scratch crate outside the workspace** (`~/scratch/…`, never committing a lockfile into the repository), build the smallest working
client (one `GET` to file with ETag/Range, one framed `POST`) against a local planeter running with the artifact store, for each candidate:
(1) `ureq` (current major) with `rustls` and the `ring` provider; (2) the same with `aws-lc-rs`; (3) the stack we already carry —
`hyper` + `hyper-util` (client) + `tokio` + `tokio-rustls` (+ `webpki-roots` or `rustls-native-certs`); (4) `reqwest` (rustls) for
reference only. For each report: **number of new crates** beyond the current 214, and which; **licenses** (run `cargo deny check` of that
scratch lockfile with our `deny.toml`: show the result and every exception it would need); **advisories** (`cargo audit` with the fresh
database); **MSRV** (does it build at 1.88.0); **native build requirements** (C compiler, assembler, cmake, perl); the count of `unsafe`
blocks in the new crates (a simple `grep` count by crate is enough, labelled as such); **root certificates** — what trust anchors the stack
uses by default (compiled-in list vs the operating system's) and how a private CA is added; **binary size** (release, stripped) and
**build time** (clean); how the requirements above are met **or what must be hand-written**; maintenance (last release date, release
frequency, number of maintainers visible on crates.io) as plain facts. Also: whether a *plain-HTTP* mode can be **compiled out** (a `https-only`
build, which is what we want the default to be).

## Also report (small)

(a) What `cargo deny`'s sources/bans rules would say of each. (b) For the preferred candidate, a **proof of the two flows** with timings:
a 100 MiB ranged download resumed after a cut connection, and a framed upload of 100 MiB, memory peak of each. (c) What a **certificate
failure** looks like to the user in each (the message text), since the helper must turn it into one plain sentence.

## Non-change scope

No crate added to the workspace, no change to any manifest or lock in the repository, no code in `crates/`, no release or version change. The
scratch crate and its lockfile are not committed; the report quotes the commands and outputs. Naming of the binary and its commands is **not**
part of this handoff.

## Acceptance criteria

The table is complete for all four candidates (a failed build is a row with the error); the proof flows ran for the preferred one; every claim
has a command and its output in the logs; the scratch area is named in the report and nothing was written into the repository except the report
and its logs under `.git-exclude/review-requests/`.

## Prohibited shortcuts

No recommendation without the table; no counting from documentation instead of from a build; no editing `deny.toml` or any manifest in the
repository to make a candidate pass (show the exception it would need instead).

## Required evidence and review-request format

The table; the commands and outputs (logs); the proof-flow timings; the certificate-failure texts; the dev team's recommendation, clearly
separated from the facts; deviations and open questions.
