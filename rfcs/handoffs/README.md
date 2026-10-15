# planeter handoffs — structure, storage and status

Companion execution documents to the RFCs, per the 5-folder lifecycle
([`../done/000-rfc-lifecycle-policy.md`](../done/000-rfc-lifecycle-policy.md)). An RFC defines and
justifies a change; a **handoff** directs its implementation and must not silently redefine its RFC.

## Layout and naming

- One directory per RFC: `NNN-slug/` (`001-foundations/` … `010-upstream-and-supply-chain-maintenance/`).
  **Every handoff implements an RFC** (workflow §6.3). Work that looks RFC-less is bound to the RFC whose
  decision it carries out, or gets one (RFC 010 covers upstream tracking and supply-chain maintenance).
  A follow-on handoff of a *done* RFC lives in that RFC's directory and is listed in its Status as
  deferred or follow-on work (RFC 000, "Granularity of transitions"). There is no `interim/`.
- Files: `<slug>-handoff-vN.md` (the handoff), `<slug>-review-request*.md` (the implementer's package
  back), `t<N>-*.md` (task-level records such as evaluations or gap catalogues). A **v2 supersedes v1
  and both stay**: the v1 keeps the reasoning that the measurement overturned.
- Every document opens with the header table — Document · Status · Basis · Audience · Scope — and the
  handoff body has three fixed parts: **task breakdown in build order** (`T1…`, each with purpose,
  applicable RFC decisions, change scope, non-change scope, required tests), the **QA checklist** drawn
  from the RFC's acceptance criteria, and the **definition of done and handback** (which gates, which
  evidence, what the review request must contain).
- Evidence is by reference: commit hashes on `main`, CI run ids, and gate outputs quoted with their exit
  codes. A claim without a reference is not evidence.

## State of handoffs

A handoff has no state of its own: it inherits its RFC's (RFC 000). **The RFC index
([`../README.md`](../README.md)) is the single source of truth.** A handoff under an RFC in `proposed/` is
not startable; under `accepted/` it is assignable; under `done/` it is history, with follow-on handoffs
listed in the RFC's Status. A handoff of a re-opened RFC is superseded by a v2 (both stay).

Standing exceptions, both explicit: the **investigation** handoffs under RFC 004 and RFC 006
(`transport-measurement-handoff-v1.md`, `ci-read-side-measurement-handoff-v1.md`) are authorized by the
owner (SCHEDULE D-7) although their RFCs are `proposed/`, because they measure and change nothing (workflow
Phases 1–2; RFC 010 D-8). The 0.52.0 re-baseline v2 under RFC 010 is implemented and conditionally approved;
its push waits on the advisory lock update.

## What a review request carries back

The implementation summary; the commits (hash → what landed); every gate with its command and exit
code, including the test run **with** prikk and the run **with prikk shimmed off PATH**; the
dependency-tree delta with `cargo deny` / `cargo audit` results when a manifest changed; deviations from
the handoff and why; known limitations; open questions for the architect. The three existing review
requests (001–003) are the worked examples.
