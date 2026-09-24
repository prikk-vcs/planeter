# Security policy

## Supported versions

planeter is pre-1.0. Only the **latest released 0.x version** receives fixes; there are no maintenance
branches. Deployments should track releases (each is a signed tag with attested Linux binaries, a
container image and the crates on crates.io — see [`docs/RELEASING.md`](docs/RELEASING.md)).

## Reporting a vulnerability

Use GitHub's private vulnerability reporting on
[prikk-vcs/planeter](https://github.com/prikk-vcs/planeter/security/advisories/new). Do not open a
public issue. Include the planeter version, the deployment shape (binary or container; reverse proxy;
SSO on or off), reproduction steps, and the impact you believe it has. Proof-of-concept material is
welcome; a working exploit is not required.

Reports are read by the project owner. Expect an acknowledgement, then a fix through an out-of-band
patch release (`X.Y.Z+1`, the same attested workflow as every release) when the report is confirmed.
Reporters are credited in the release notes unless they ask otherwise.

Issues in **prikk** itself (the version-control system planeter hosts) go to prikk's own security
policy; issues in how planeter *drives* prikk are planeter's.

## Scope and the security model, briefly

planeter's threat model is a first-class deliverable:
[`docs/src/planeter-03-threat-model-v0.1.md`](docs/src/planeter-03-threat-model-v0.1.md) (v0.3). The
properties a report is measured against:

- **The forge holds no history-signing key.** A compromised planeter can deny history but cannot forge
  prikk-verified history. The default build contains no forge-seal path; CI proves the symbol is absent.
- **prikk runs in a sandbox** (bubblewrap: no network, repository-directory-only writes, cleared
  environment, wall-time bound) and is the source of truth for every repository fact.
- **One default-deny `authorize()`** on every read and write path; no existence leak for hidden
  repositories.
- **Rendered content is sanitized** under a strict CSP; raw bytes are served only as inert downloads
  from an isolated origin.
- **Outbound requests pass an egress guard** (public unicast only, HTTPS only, no redirects, size and
  time bounds) and a confined `curl`.
- **planeter speaks plain HTTP behind a TLS-terminating reverse proxy** it must be told about; it
  refuses a non-loopback bind without one.

Known, documented residual risks (threat model §Residual risks): no second factor on local accounts
(use an OIDC provider that enforces MFA), and login throttles that are per process (rate-limit
`/login` at the proxy in multi-replica deployments). These are not vulnerabilities to report, but
improvements to them are welcome.
