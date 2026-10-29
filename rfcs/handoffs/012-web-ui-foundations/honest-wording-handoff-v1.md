# RFC 012 follow-on — Honest wording: the queued badge, its explanation, and the closed line — Implementation Handoff (v1)

| | |
|---|---|
| Document | The owner delegated this decision to the architect (D-50). Two owner-approved texts say something the product no longer means: **"Approved, awaiting signature"** (badge) and **"This change was approved on this server…"** (`explain.queued`). Nobody approves a patch that a keyless forge holds; *approved* now belongs to a reviewer's decision about a revision (part 6), and *change* belongs to a recorded unit of history. On one proposal page the same word would mean two things — the confusion the owner forbids. This handoff rewords them, and makes the closed line say "Reason: …". |
| Status | **Done, reviewed Approved (2026-10-30; commits `2dca975`).** (Originally: **Assigned** (2026-10-29). |
| Basis | `crates/planeter-web/src/catalog/{en,ja}.rs` (`assurance.queued`, `explain.queued`, `proposal.closed_line`); `docs/glossary.md` (lines of the *Approved, awaiting signature* row, `explain.queued` row, and §5.1/§5.2 Japanese rows); `tests/retired_words.rs`, `tests/glossary*.rs`, `tests/plain_words.rs`; IS-47 (`queued` never means sealed); RFC 005 D-10 (the machine value `forge-approved-unsealed` stays — it is in the released API). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The texts (architect's wording; the Japanese is a draft for the owner and the colleague).**

  | Key | English (before → after) | Japanese draft (before → after) |
  |---|---|---|
  | `assurance.queued` (badge) | "Approved, awaiting signature" → **"Held here, not signed in"** | 承認済み・署名待ち → **ここで保持中・未署名** |
  | `explain.queued` | "This change was approved on this server. A maintainer has not yet signed it into the project's history, so it is not part of the verified record." → **"This patch is held on this server. A maintainer has not signed it into the project's history, so it is not part of the verified record."** | この変更は、このサーバーで承認されました。メンテナーがまだ…署名していないため、… → **このパッチは、このサーバーで保持されています。メンテナーがプロジェクトの履歴にまだ署名していないため、検証済みの記録には含まれていません。** |
  | `proposal.closed_line` | "Closed: {reason}" → **"Reason: {reason}"** | 閉じています：{reason} → **理由：{reason}** |

  Why: *held* is what is true (the server holds it, accepted but unsigned — ledger PK-58, U-10); *not signed in* is the trust-bearing half and keeps the meaning of the old badge's second half; nothing says "approved"; "patch" is the glossary word for the thing; no promise that someone will sign it (a keyless forge cannot, and D-8 is not built). The badge stays a warning-class (`warn`) badge and stays a trust label: its meaning (not part of the verified record) is unchanged. The machine value `forge-approved-unsealed`, the CSS class, the keys' names and the API are **unchanged** (a catalog key keeps its name; only its text changes).
- **T2 — Glossary and tests follow.** `docs/glossary.md`: the row, the message table (§2 and §5.2) and the Japanese row; the history of the old wording in one line ("2026-10-29: reworded, D-50"). The retired-words test (English): "Approved, awaiting signature" and "was approved on this server" become **retired phrases** (planted cases both ways, so they cannot come back); the Japanese forbidden list gains 承認済み・署名待ち; the existing rule "approved stays out of any sentence about a proposal that is not a reviewer's decision" stays. The plain-words and glossary tests are updated to the new strings, **not weakened**. The Japanese cells are marked "draft — reworded, owner and colleague to confirm".
- **T3 — Where the badge is shown.** Check every page that shows `AssuranceQueued` (the proposal detail, the history/change pages) renders sensibly with the new text (it is longer: wrapping at narrow widths, the WCAG checker, the stylesheet test). Add the new Japanese cells to the next addendum for the reviewers (three rows, marked REWORDED, with the old text beside).

## Non-change scope

No change to any assurance value, class, key name, API field or colour; no other text; no new message; no version or release change; the other owner-approved texts stay.

## Acceptance criteria

The three texts changed in both catalogs; retired-phrase tests (planted cases) pass; glossary and tests updated; pages pass the WCAG checker and plain-words tests in both languages; every gate green (fmt, floor, pin, clippy, both test runs, ENF-2, doc; deny/audit as they are while IS-54 is open), CI.

## Prohibited shortcuts

No weakening of a test to pass; no change of a key's name or of the machine value; no use of *approved* or *change* in the new texts.

## Required evidence and review-request format

Commits (hash → what); the before/after table as built; the HTML of one proposal page with a held patch and one closed proposal, both languages, with the checker's result; every gate with command and exit code; deviations and open questions.
