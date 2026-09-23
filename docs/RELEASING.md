# Releasing planeter

planeter releases an **application**: a bare version tag → `release.yml` → attested binaries + a
container image → a GitHub release. The nine `planeter-*` crates are **also** published to crates.io
after the release run is green, as an install path (`cargo install planeter`); `planeter-runner` is not
published until M4. **Tagging and publishing are owner-authorized.**

## Before the tag (the release-candidate commit)

1. Gates green on `main`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked
   -- -D warnings`, `cargo test --workspace --locked`, `cargo deny check`, `cargo audit`,
   `tools/enf2-symbol-check/check.sh`.
2. `CHANGELOG.md` has a `## [X.Y.Z]` section for the release (the workflow refuses to publish
   without one).
3. `Cargo.toml` `[workspace.package] version = "X.Y.Z"` equals the tag (the workflow's first gate).
4. The prikk floor is current: `PRIKK_VERSION` in `release.yml` and `Dockerfile` match
   `MIN_PRIKK_VERSION` in `crates/planeter-prikk/src/cli.rs` (dependency-ledger PK-26).

## The tag (owner)

```sh
git tag -s X.Y.Z -m "planeter X.Y.Z"
git push origin X.Y.Z
```

A signed tag, bare version (no `v`), matching the ecosystem convention. The push triggers
`release.yml`.

## What the workflow does

1. **gates** — re-runs every gate on the tagged commit and checks tag == version.
2. **build** — `planeter` for `x86_64-unknown-linux-gnu` (ubuntu-latest) and
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
```

## Deploying

- **Binary:** needs `prikk` (≥ the floor in `build-info.txt`) and `bwrap` on `PATH`. Configure with
  `PLANETER_ADDR`, `PLANETER_REPOS_ROOT`, `PLANETER_DB`, `PLANETER_CONTENT_ORIGIN`.
- **Container:** persist `/var/lib/planeter` (repos + SQLite). bubblewrap needs user namespaces
  inside the container: typically `--security-opt seccomp=unconfined` (or a seccomp profile allowing
  `unshare`/`clone` with `CLONE_NEWUSER`) and, on kernels that restrict unprivileged user namespaces,
  `--cap-add SYS_ADMIN`. Serve raw content from a **distinct origin** (`PLANETER_CONTENT_ORIGIN`) in
  front of the same server.
- **Format 7:** keep the serving prikk at or above the on-disk format your hosted repositories carry;
  planeter never upgrades a repository's format on its own.

## Release log

- **0.1.0 (2026-09-23).** Gates, both builds and the container image passed; `publish` failed twice
  because an unfiltered `download-artifact` also fetched Docker's `*.dockerbuild` build-record
  artifact, which could not be downloaded (fixed on `main` in `fedf904` with `pattern: planeter-*`).
  The release was created by the fallback below from the run's own artifacts after `sha256sum -c`
  and `gh attestation verify` both passed. crates.io: nine crates, dependency order; the new-crate
  rate limit (burst of 5, then ~1 per 10 min) paced the last four.

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
