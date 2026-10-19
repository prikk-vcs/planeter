# Releasing planeter

planeter releases an **application**: a bare version tag → `release.yml` → attested binaries + a
container image → a GitHub release. The nine `planeter-*` crates are **also** published to crates.io
after the release run is green, as an install path (`cargo install planeter`); `planeter-runner` is not
published until M4. **Tagging and publishing are owner-authorized.**

## Before the tag (the release-candidate commit)

1. **Immediately before the tag** (an advisory can appear between the release candidate and the tag, as RUSTSEC-2026-0373 did for 0.3.0): refresh the advisory database and run `cargo deny check` and `cargo audit` on the exact commit to be tagged.
   Gates green on `main`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked
   -- -D warnings`, `cargo test --workspace --locked`, `cargo deny check`, `cargo audit`,
   `tools/enf2-symbol-check/check.sh`, and the two maintenance gates `cargo run --locked -p
   planeter-floor-check` and `cargo run --locked -p planeter-pin-check` (RFC 010 D-2, D-5). The
   supply-chain pair may also be run as a checkpoint with the manual `supply-chain.yml` workflow.
2. `CHANGELOG.md` has a `## [X.Y.Z]` section for the release (the workflow refuses to publish
   without one).
3. `Cargo.toml` `[workspace.package] version = "X.Y.Z"` equals the tag (the workflow's first gate).
4. The prikk floor is current: `PRIKK_VERSION` in `release.yml` and `Dockerfile` match
   `MIN_PRIKK_VERSION` in `crates/planeter-prikk/src/cli.rs` (dependency-ledger PK-26); `floor-check`
   enforces the equality, the review of which version is current is yours.
   The declared minimum compiler (`rust-version` in `Cargo.toml`) is the `msrv` job's toolchain in
   `ci.yml` (`pin-check` enforces it); the `msrv` job is green on the release-candidate commit. If a
   change raised the minimum (a language feature, a dependency), it says so in the CHANGELOG.
5. **Review the pins against upstream releases and advisories before tagging** (RFC 010 R-1): every
   `uses:` SHA and the base-image digest are immutable and do not refresh themselves. For each, compare the
   pinned tag with the upstream's latest and read the advisories; an update is a reviewed change that names
   the upstream diff, and `pin-check` must stay green.

## The tag (owner)

```sh
git tag -s X.Y.Z -m "planeter X.Y.Z"
git push origin X.Y.Z
```

A signed tag, bare version (no `v`), matching the ecosystem convention. The push triggers
`release.yml`.

## Dry run (before a real tag, and after any change to a workflow or the Dockerfile)

`release.yml` can be dispatched by hand: **Actions → Release → Run workflow** on `main`, or
`gh workflow run release.yml --ref main -f dry_run=true`. A dispatch is **always a dry run**
(`dry_run` defaults to true; `dry_run: false` is refused): it runs the gates, builds both binaries with
`PLANETER_GIT_COMMIT` set, writes `build-info.txt`, and **builds** the `linux/amd64` + `linux/arm64`
image. It does **not** log in to the registry, push or attest an image, attest the tarballs, create a
GitHub release or touch crates.io (the workflow never does; see "crates.io" below). The tag-vs-version
check is skipped (there is no tag). Download the run's `planeter-*` artifacts to read
`build-info.txt` and run the binary's `--version`. What a dry run cannot show: registry authentication,
the attestations, and the notes assembled from a tag's CHANGELOG section.

## What the workflow does

1. **gates** — re-runs every gate on the tagged commit and checks tag == version.
2. **build** — `planeter` for `x86_64-unknown-linux-gnu` (ubuntu-24.04) and
   `aarch64-unknown-linux-gnu` (native arm runner); tarball + `.sha256` + `build-info.txt`; a
   **build-provenance attestation** for each tarball.
3. **container** — `ghcr.io/prikk-vcs/planeter:<tag>` and `:latest`, `linux/amd64` + `linux/arm64`,
   built from the prebuilt binaries (never compiled in Docker) with a checksum-verified prikk
   release and bubblewrap baked in; the image is attested too.
4. **publish** — the GitHub release with the tarballs, checksums, build-info, and notes taken from
   the tag's `CHANGELOG.md` section plus the runtime prerequisites.

Only Linux targets ship: the sandbox is bubblewrap, which is Linux-only, and planeter does not make
platform claims it cannot run (prikk RFC 107's lesson).

## Verifying a release

```sh
sha256sum -c planeter-x86_64-unknown-linux-gnu.tar.gz.sha256
gh attestation verify planeter-x86_64-unknown-linux-gnu.tar.gz --repo prikk-vcs/planeter
gh attestation verify oci://ghcr.io/prikk-vcs/planeter:X.Y.Z --repo prikk-vcs/planeter

# the build identifies itself (RFC 010 D-6): the commit is the tag's and the floor is the release's
tar xzf planeter-x86_64-unknown-linux-gnu.tar.gz && ./planeter --version
docker run --rm ghcr.io/prikk-vcs/planeter:X.Y.Z --version
```

## Deploying

- **Binary:** needs `prikk` (≥ the floor in `build-info.txt`), `bwrap` and **`prlimit`** (util-linux) on
  `PATH`, plus `curl` if SSO is enabled. `timeout` (coreutils) is no longer used. Configure with
  `PLANETER_ADDR`, `PLANETER_REPOS_ROOT`, `PLANETER_DB`, `PLANETER_CONTENT_ORIGIN`, and
  **`PLANETER_TRUSTED_PROXIES`** (the TLS-terminating proxy's addresses/CIDRs, comma-separated).
  planeter speaks plain HTTP: it **refuses a non-loopback bind unless trusted proxies are set**, takes
  the client IP from `X-Forwarded-For` only when the TCP peer is a trusted proxy, and marks cookies
  `Secure` (so the proxy must serve HTTPS).
- **Raw content origin (IS-16):** raw repository bytes are always inert downloads (attachment,
  `nosniff`, a sandbox CSP). *Isolation from the application's origin* holds only when
  `PLANETER_CONTENT_ORIGIN` names a **different host** (a different port on the same host is not
  isolation: cookies ignore the port). Then the raw route answers **only** for requests whose effective
  host is that host, and the application's own host answers `404` exactly as for an unknown route. The
  effective host is the `Host` header, or the last `X-Forwarded-Host` value **only** when the TCP peer is
  in `PLANETER_TRUSTED_PROXIES`. Start-up: a non-loopback `PLANETER_ADDR` whose content origin has the
  application's own host (the default is `http://<PLANETER_ADDR>`) **refuses to start**; on a loopback
  bind it logs one warning and the raw route answers on the application host. There is no override: a
  second host name for the same proxy is all it takes. **Proxy requirement:** preserve the original
  `Host`, or send `X-Forwarded-Host` and list the proxy in `PLANETER_TRUSTED_PROXIES`; a proxy that
  rewrites `Host` to the upstream's name makes the content host unreachable. **Breaking in 0.3.0:** a
  production-shaped deployment with no distinct content host no longer starts, and raw bytes are no longer
  served from the application's host when a distinct origin is configured.
- **API clients and raw bytes (IS-33):** an API client asking the application's host for
  `…/raw?path=…` gets `404` with the code `raw_elsewhere`. It should send the same request to the content
  origin, which the OpenAPI document (`/api/v1/openapi.json`) names in the `servers` entry of the raw
  operation; the file page's own link already points there. `raw_elsewhere` depends only on the host
  asked, never on the repository or the caller, so it says nothing about whether a repository exists.
- **Cache directives and HSTS (IS-29):** the app sets its own `Cache-Control` (`private, no-cache` and
  `Vary: Cookie, Authorization` on dynamic responses, `no-store` on sign-in and sign-out, a one-hour
  `public` lifetime only on the stylesheet), so the proxy needs no cache rules of its own and must not
  override them. When `PLANETER_TRUSTED_PROXIES` is set the app also sends
  `Strict-Transport-Security: max-age=15552000` (180 days; no `includeSubDomains`, no `preload`): the
  proxy must therefore serve HTTPS for the whole host. A proxy that adds its own HSTS header should
  replace the app's value, not append a second one. API errors are JSON (`{code, message, hint}`); a proxy's own
  error pages are not.
- **Memory ceiling and child lifetime (RFC 011 D-2, D-5):** every prikk call runs as `bwrap` (the
  server's direct child, `--die-with-parent`); each read's memory ceiling (1.5 × its cost, below) is
  applied as `prlimit --data=<N> --` in front of that command. **`prlimit` is a start-up prerequisite:** planeter
  checks `prlimit --version` once and refuses to start with a message naming `prlimit` and util-linux
  (it is `Essential` in Debian, so it is in the container image; a minimal distribution may need
  `util-linux` installed). The wall-time bound (120 s) is enforced by the server, which `SIGKILL`s the
  call at the deadline (no SIGTERM grace); a **killed server no longer leaves its children running**
  (before, `timeout` → `bwrap` outlived it until the call ended). A call that hits its ceiling is a `503`
  (`unavailable`) and logs one line naming the verb and the ceiling; it is never reported as a
  verification result.
- **Sign-in concurrency (IS-24, IS-31):** password verification (Argon2id) runs off the async workers
  behind its own gate, separate from the prikk one. **`PLANETER_AUTH_CONCURRENCY`** is the most
  verifications at once (positive integer; default `min(CPUs, 4)`); **`PLANETER_AUTH_QUEUE_WAIT_SECS`**
  is how long a sign-in waits for a slot before `503` and `Retry-After` (positive integer; default 5).
  Every password sign-in costs one verification, an unknown account's against a dummy hash made at start,
  so neither the response time nor the `503` reveals whether an account exists. Token and session checks
  take no permit. Both values are logged once at start.
- **Concurrency (IS-4):** read-path prikk work runs off the async workers behind a gate.
  **`PLANETER_PRIKK_CONCURRENCY`** is the most prikk subprocesses at once (positive integer; default
  `min(CPUs, 8)`); **`PLANETER_PRIKK_QUEUE_WAIT_SECS`** is how long a request waits for a slot before it
  is refused with `503` and `Retry-After` (positive integer; default 10). Both values are logged once at
  start. A low cap starves legitimate bursts; raise it with the host's memory in mind (prikk's figures
  are several times the repository's content, dependency ledger PK-40).
- **Read memory budget (RFC 011 D-1, IS-36):** the process-count gate above bounds how many prikk
  processes run, not how much memory they use (a read's memory follows the *repository's size*, not the
  answer's: eight concurrent `verify` calls on a 1 GiB repository held 16 GB). Reads are therefore also
  admitted by **estimated memory**: each read declares a cost from the repository's store size S (a fresh
  walk of its `.prikk` at every request) and the verb — `tree`/`cat` 2.1 × S, `verify` 4.1 × S, `show`
  3.6 × S, the API's `file` view 2.1 × S plus 12.4 MiB, `log`/refs a fixed 16 MiB; none less than 16 MiB
  — and must fit the budget **and** a process slot within the one bounded wait
  (`PLANETER_PRIKK_QUEUE_WAIT_SECS`). **`PLANETER_PRIKK_MEMORY_BUDGET_MB`** (positive integer, MiB) sets
  the budget; unset, it is **a quarter of the memory the process may use** (its cgroup's `memory.max`
  when set, otherwise physical memory), **at least 256 MiB and at most 8 GiB**. A value you set is never
  capped. The start-up log states the budget, where it came from and the variable to change it.
  **Sizing rule: budget ≥ 4.1 × the largest repository you expect to `verify`** (a 1 GiB repository
  needs ≈ 4.2 GiB) — **more exactly, the heavy pool is what a `verify` is judged against** (see the next
  paragraph), so: **budget × 7/8 ≥ 4.1 × the largest repository you `verify`**. Answers: a read that does
  not fit **its own pool** is refused at once with `422 too_large` (never queued; "ask the operator to
  raise the read memory budget" — this variable); a
  wait that ends is `503 busy` + `Retry-After`. Each prikk call also runs under a memory **ceiling** of
  1.5 × its cost (never above the budget), so a wrong estimate costs one failed call (`503
  unavailable`, one log line naming the verb and the ceiling), not the host. Every refusal logs one
  line (verb, cost, budget; no path, no repository name). The repository page shows its `verify` badge
  from a separate admission: a repository whose `verify` does not fit still gets its page, with
  "verify unavailable". The costs are measured on a prikk release (0.52.0) and are re-measured at every
  floor change (`docs/UPSTREAM.md`).
- **Two pools (RFC 011 refinement R-1):** the budget is split so that a cheap read is never queued behind
  a large one that waits for memory (one FIFO would park a `log` behind a waiting `verify`, and refuse it
  `busy` in a race though it fits). The **light pool** is `max(budget / 8, 64 MiB)` (never more than half
  the budget) and serves only the floor class of reads — those costed at most 32 MiB: `log`, refs, a
  `show` of a small patch, the `tree` of a tiny repository; the **heavy pool** is the rest and serves every
  other read. The slice is derived, not configurable, and there is no borrowing between pools: a floor
  read does not use heavy memory, and a heavy read does not use light memory. A read is judged against
  **its own pool**, so the heavy pool is the effective whole-budget bound for heavy reads (a 4 GiB budget:
  512 MiB light, 3.5 GiB heavy). The start-up log states both sizes; process slots stay shared.
- **Bounded content (RFC 011 D-3, D-4):** the API's `file` view is bounded at 4 MiB (a larger file is
  `422 too_large`), as is the UI's file page; **the raw route keeps its 25 MiB cap** (unchanged since before
  0.3.0; above it `422 too_large` with its own hint instead of the former `503 unavailable`). The
  repository page's README preview stays at 1 MiB.
- **SSO (OIDC):** set `PLANETER_OIDC_ISSUER`, `PLANETER_OIDC_CLIENT_ID`, `PLANETER_OIDC_REDIRECT_URI`
  (and `PLANETER_OIDC_CLIENT_SECRET` for a confidential client). The provider is reached only through
  the egress guard and a bubblewrap-confined **`curl`** (add `curl` to the runtime prerequisites; it is
  in the container image). Link each account to its provider identity with the account store's
  `link_oidc(issuer, subject, user)` — an unlinked identity is refused; planeter never auto-creates
  accounts from SSO.
- **Container:** persist `/var/lib/planeter` (repos + SQLite). bubblewrap needs user namespaces
  inside the container: typically `--security-opt seccomp=unconfined` (or a seccomp profile allowing
  `unshare`/`clone` with `CLONE_NEWUSER`) and, on kernels that restrict unprivileged user namespaces,
  `--cap-add SYS_ADMIN`. Serve raw content from a **distinct host** (`PLANETER_CONTENT_ORIGIN`) in
  front of the same server (see *Raw content origin* above).
- **Format 7:** keep the serving prikk at or above the on-disk format your hosted repositories carry;
  planeter never upgrades a repository's format on its own.

## Release log

- **0.1.0 (2026-09-23).** Gates, both builds and the container image passed; `publish` failed twice
  because an unfiltered `download-artifact` also fetched Docker's `*.dockerbuild` build-record
  artifact, which could not be downloaded (fixed on `main` in `fedf904` with `pattern: planeter-*`).
  The release was created by the fallback below from the run's own artifacts after `sha256sum -c`
  and `gh attestation verify` both passed. crates.io: nine crates, dependency order; the new-crate
  rate limit (burst of 5, then ~1 per 10 min) paced the last four.

- **0.1.1 (2026-09-23).** The workflow completed unaided end to end — gates, both builds, the
  container image, and `publish` (the `planeter-*` artifact filter fixed the 0.1.0 defect). A release
  asset's checksum and build-provenance attestation were verified after the fact; the nine crates were
  published to crates.io in dependency order (30 seconds; no new-crate rate limit applies once the
  crates exist).

- **0.2.0 (2026-09-24).** Preceded by a full documentation audit (README, ROADMAP, RFC index, threat
  model, crate docs). The workflow completed unaided: gates, both builds, the container image and
  `publish`; the release notes carry the new prerequisite line (curl for SSO). The x86_64 tarball's
  checksum and build-provenance attestation verified after the fact; the nine crates were published in
  dependency order (36 seconds) and `cargo install planeter --version 0.2.0 --locked` builds and runs.
  The image attestation cannot be verified anonymously while the ghcr.io package is private (owner
  action pending).

## If a job fails

The workflow is idempotent up to `publish`: re-run the failed job with `gh run rerun <run-id> --failed`
(the tag's workflow file and the run's attested artifacts are reused). If `publish` cannot be
repaired, download the run's `planeter-*` artifacts (`gh run download <run-id> -p 'planeter-*'`) and
create the release by hand with the same asset set and the tag's `CHANGELOG.md` section; the
build-provenance attestations are bound to the tarball digests, so `gh attestation verify` still
holds. Never move or recreate a pushed tag.

## Security releases

An advisory or a threat-model control failure triggers an out-of-band patch release
(`X.Y.Z+1`) through the same workflow.

## crates.io (after the release run is green)

crates.io requires every intra-workspace dependency to carry a version, and rejects a crate whose
dependencies are not yet published — so publish in dependency order, each with the workspace version:

```sh
for c in planeter-prikk planeter-store planeter-core planeter-auth \
         planeter-web planeter-transport planeter-ci planeter-registry planeter; do
  cargo publish -p "$c" --locked
done
```

Dry-run only the leaves (`planeter-prikk`, `planeter-store`) before the first real publish: a dry-run
of a crate whose internal dependencies are unpublished fails by construction. The crate name
`planeter` is reserved by the owner (a `0.0.0` placeholder, 2026-09-16).
