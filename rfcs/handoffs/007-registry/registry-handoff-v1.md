# RFC 007 — Registry — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 007 (Package and artifact registry). |
| Status | Inherited from RFC 007 — **Accepted**. |
| Basis | [`../../accepted/007-registry.md`](../../accepted/007-registry.md) (D-1…D-7); RFC 002 (publish authz), RFC 006 (CI publish token, provenance); `STD-5`, `SEC-4`, `OPS-1`. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | **Phase A5 → M5.** `planeter-registry`. Orthogonal to prikk (touches no prikk verb). |

## Task breakdown (PR plan)

- **T1 — OCI Distribution + OCI artifacts (D-1).** Implement the OCI Distribution Specification
  (push/pull for `docker`/`podman`/containerd); support at least one **OCI artifact** type (e.g. an
  SBOM or a Helm chart) as the substrate.
- **T2 — First language registry by demand (D-2).** Implement one language registry (the first users
  need) as a faithful protocol implementation, so its native tool works with only URL+token changed.
- **T3 — Scoped, authenticated publishing (D-3).** Publishing always authenticated (RFC 002); a human
  PAT/OAuth token or a **CI job-scoped token** (RFC 006 D-4) limited to its authorized target. Pulls
  follow visibility (public anonymous). Tests: unscoped/read-only/wrong-repo token cannot publish.
- **T4 — Immutability (D-4).** A published `name@version`/digest is **write-once**; re-publish is
  refused; **yank** marks uninstallable without mutating/deleting bytes. Tested.
- **T5 — Object storage (D-5).** Bytes digest-addressed in S3-compatible object storage; metadata in
  `planeter-store`; the app disk is never the artifact store.
- **T6 — Integrity custodian (D-6).** Store and verify cosign/Sigstore signatures and SLSA/in-toto
  provenance/SBOMs attached by publishers/CI; surface the result; **hold no signing material**.

## QA checklist (from RFC 007 acceptance)

- [ ] OCI push/pull works auth-scoped; ≥1 OCI artifact type stored; ≥1 language registry works natively.
- [ ] **Publish authz (tested):** unscoped/read-only/wrong-repo token blocked; CI job token publishes only
      to its authorized target.
- [ ] **Immutability (tested):** re-publish refused; yank works without mutating bytes.
- [ ] Bytes digest-addressed in object storage (not app disk).
- [ ] **Integrity (tested):** an attached signature/provenance is stored and verified and surfaced;
      planeter holds no signing material.
- [ ] Gates green; `planeter-03` updated if a new data flow was introduced.

## Definition of done & handback

Done when the checklist is green — **M5 (0.5.0)**. Return the review-request package naming the PRs, the
object-storage backend, and the first language registry chosen. Track B's **RFC 009** (durability) and
the two prikk-side asks remain the path to 1.0. All v0.x; 1.0 is the owner gate.
