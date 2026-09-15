# RFC 007 — Package and artifact registry

**Status.** Proposed (2026-09-15) — packages for M5. Draft for review; on acceptance the implementer
builds the registry per the handoff. Defines *how planeter hosts build outputs* — against the protocols
their own tools already speak, authenticated and immutable, with the bytes in object storage.
Handoff: forthcoming (`../handoffs/007-registry/`).
**Tracks.** ROADMAP Phase A5 → M5 (Track A). Requirements `CAP-7`, `STD-5`, `SEC-4`; external design
`REG-01/02`, `HOOK`; internal design `RG-1/2`, `CIO-4`; forge-commons *Package and Artifact Registries*,
*Integrity*. Consumes RFC 002 (publish authz) and RFC 006 (CI publishes via a job-scoped token).
**Touches.** `planeter-registry` (the registry protocols + object storage). **Not here:** the VCS/prikk
boundary — a registry is orthogonal to history (`RG` touches no prikk verb); CI (RFC 006) *produces
into* this.

## Summary

A forge hosts the *build outputs* — container images, language packages, release binaries — so the place
that reviews the code also distributes the artifact. There is no single "package registry" protocol;
each ecosystem speaks its own, and this is where forge-commons' *lean* discipline bites: **support the
formats users need, and treat each additional one as a protocol you promise to keep correct.** planeter
leads with the one neutral standard (**OCI Distribution**) and adds language registries by demonstrated
demand, all under the same rules — authenticated scoped publishing, immutable published versions, and
bytes in object storage. Unlike everything in RFCs 001–006, the registry **does not touch prikk**: it is
forge-owned artifact hosting, cleanly separate from the VCS.

## The constraints that scope this design

- **No universal registry protocol exists** (survey/forge-commons): OCI is neutral for containers and,
  via OCI artifacts, a substrate for more; every language ecosystem (npm, Maven, PyPI, NuGet, Cargo, …)
  is its own protocol. Each added format is real, ongoing surface.
- **Immutability is what the dependency world assumes** (`STD-5`): a published `name@version` must never
  silently change; yank (mark uninstallable) is legitimate, mutate is not.
- **Publishing is always authenticated; pulls follow visibility** (`SEC-*`): an unscoped publish token is
  the classic supply-chain foothold.
- **Bytes are large and numerous** (`OPS-1`): they belong in **object storage** addressed by digest, not
  on the application disk.
- **The forge is a custodian of integrity, not a key-holder** (`SEC-4`): it stores and verifies
  signatures/provenance; it does not sign artifacts for users.

## Decisions

- **D-1 — OCI Distribution first; OCI artifacts as the substrate.** Implement the **OCI Distribution
  Specification** for container images and prefer **OCI artifacts** for further types (Helm charts,
  SBOMs, signatures, WASM) rather than standing up bespoke registries — one well-understood protocol over
  many half-maintained ones.
- **D-2 — Language registries by demonstrated demand.** Add npm, Maven, PyPI (Simple Repository API),
  NuGet, Cargo, … each as a **faithful implementation** of that ecosystem's protocol, so `docker push` /
  `npm publish` / `mvn deploy` / `pip install` / `cargo publish` work against planeter with only a
  registry URL and a token changed. **Add by demand, not as a checklist** — each is a maintenance
  obligation (`NG` lean discipline).
- **D-3 — Authenticated, scoped publishing (RFC 002).** Pulls may be public or private per repository
  (following visibility, RFC 002 D-5); **publishing is always authenticated** with a scoped token — a
  human PAT/OAuth token or a **CI job-scoped token** (RFC 006 D-4) limited to its authorized registry. A
  read-only or wrong-scope token cannot publish.
- **D-4 — Immutable published versions.** A published `name@version` (or OCI digest) is **write-once**;
  **yank** marks it uninstallable for new resolves; **mutation is refused**. This is a hard invariant of
  the registry, tested.
- **D-5 — Digest-addressed object storage (OPS-1).** Artifact bytes live in **S3-compatible object
  storage**, addressed by digest, with the registry metadata in `planeter-store`. The application disk is
  never the artifact bottleneck; storage scales out independently.
- **D-6 — Integrity: store and verify, never hold keys (SEC-4).** The registry **stores and verifies**
  cosign/Sigstore signatures and SLSA/in-toto provenance and SBOMs alongside the artifacts (forge-commons
  *Integrity*), and surfaces the result. **Generation** is the publisher's / CI's doing with their own
  materials (RFC 006 D-8); planeter is the custodian, consistent with holding no signing key anywhere
  (`INV-2`'s spirit extends to artifacts).
- **D-7 — Orthogonal to prikk.** The registry references no prikk history and invokes no prikk verb;
  it is forge-owned artifact hosting. (A release may *link* a registry artifact to a prikk-sealed tag by
  id — a cross-reference, RFC 005 `DM-4` — but the artifact bytes are the registry's, not prikk's.) This
  keeps `LAY-3` (only `planeter-prikk` touches prikk) trivially true for the registry.

## What "done" means (acceptance criteria)

- **OCI Distribution** works: `docker`/`podman` push and pull against planeter, auth-scoped; at least one
  **OCI artifact** type stored (D-1).
- At least one **language registry** (the first by demand) works with its native tool (D-2).
- **Publishing authz (tested):** an unscoped/read-only/wrong-repo token cannot publish; a CI job-scoped
  token publishes only to its authorized target (D-3).
- **Immutability (tested):** re-publishing an existing `name@version`/digest is refused; yank marks
  uninstallable without deleting/mutating bytes (D-4).
- **Storage:** bytes are digest-addressed in object storage; the app disk is not the artifact store (D-5).
- **Integrity (tested):** a cosign signature / provenance attached by a publisher is **stored and
  verified** and surfaced; planeter holds no signing material (D-6).
- Threat model re-verified: publishing authz and immutability (`STD-5`), no key-holding (`SEC-4`);
  `planeter-03` updated if a new data flow was introduced.

## Alternatives considered

- **A bespoke registry per artifact type.** Rejected (D-1): OCI artifacts are the neutral substrate;
  reach for a bespoke protocol only when an ecosystem's own tooling requires it (D-2).
- **Mutable "latest"-style overwrites of a published version.** Rejected (`STD-5`/D-4): the dependency
  world assumes immutability; mutation is a supply-chain hazard.
- **Sign artifacts on the users' behalf with a forge key.** Rejected (`SEC-4`/D-6): planeter is a
  custodian; signing is the publisher's/CI's, mirroring the forge-holds-no-key discipline for history.
- **Artifacts on the application filesystem.** Rejected (`OPS-1`/D-5): object storage from the start.

## Open questions & dependencies

- **OQ-4 (owner)** — which language registries are in scope for v0 beyond OCI + the first-by-demand;
  each is an ongoing obligation (D-2).
- **RFC 006 (CI)** — the job-scoped publish token (D-3) and provenance generation (D-6) come from CI;
  the seam is defined there.
- Object-storage backend choice and retention/GC policy are operational (`OPS-*`), settled in the
  handoff; the digest-addressed contract (D-5) is fixed here.

## Sequencing & handoff

Builds on RFC 002 (publish authz) and RFC 006 (CI publishing). On acceptance, the architect writes
`handoffs/007-registry/` (the OCI Distribution implementation, the first language registry, the
scoped-publish + immutability enforcement, the object-storage layout, the signature/provenance
store-and-verify, and the tests above). Delivering this reaches **M5 (0.5.0)**. Then **RFC 009
(hosted-format durability)** — Track B, owner-gated — closes the path to 1.0. All v0.x; 1.0 is the owner
gate.
