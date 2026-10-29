# `extract` — the driver (RFC 005 part 5b)

What this is: the part of planeter that asks prikk for **the files of one point of history** (a ref, a block, or a pending
claim) and gets them as a directory. It is a driver only. No page, no API route and no job uses it yet: the review UI (RFC 005
part 6) and the checkpoint-refresh job (RFC 006) are built on it. Code: `planeter-prikk::extract`; cost: `planeter-core::admission`.
prikk ≥ 0.54.0 (`MIN_PRIKK_VERSION`); measured on 0.54.1; the refusal ids re-checked on 0.55.0.

## The destination is ours to make safe

prikk 0.54.x checks only that the destination does not exist yet. A path **inside the repository** (even under `.prikk/`) is
accepted, and a copy left in the worktree would be taken as content by the next `commit --from-worktree`. (prikk 0.55.0 refuses it.)
So the driver never takes a destination from its caller:

* It makes a **per-call working directory** `<area>/work/<id>/` (`ExtractWorkdir`), with `out/` and, inside it, `tree` — the
  destination, which does not exist until prikk makes it. The `<id>` is 96 random bits.
* It refuses, before any prikk call and with nothing created, an area that resolves (links followed) to the repository root,
  inside it, or to a directory that contains it; a relative area; an area whose `work` is a link. It checks again at the call,
  against the repository the driver is bound to.
* The sandbox mounts **only `out/`** read-write (plus the repository, as for every call). prikk cannot write elsewhere.
* Dropping the guard removes the whole directory, with everything in it.

## What comes out is untrusted

File names and contents are whatever the pusher committed; a symlink in the tree may point anywhere. The driver returns the path
and the report and **never reads, walks or follows anything in the tree** (a test puts a link to a pipe nobody writes to in the
tree and shows that the call returns). Any reader of the tree — the future comparison page — must treat every name and every
byte as hostile, must not follow links, and must not execute anything. The tree is removed when the guard goes.

## What prikk says, and what the driver makes of it

A success is `extract-report-v1` (`point`, `point_kind`, `target_block_id`, `claim_parent_block_id`, `provisional`, `written`,
`file_count`, `total_bytes`, `largest_file_bytes`). `provisional: true` means no checkpoint the repository itself verified was in
reach, so the files match the history that was replayed but **nothing has checked that history**. A refusal is `refusal-v1` on
stdout (the exit status is 1, the message is also on stderr); a crash, a timeout and a memory ceiling are the usual typed
`PrikkError`. The readers are strict: a missing or mistyped field, another `schema_version`, a report that disagrees with the call
(`written` for a plan, a claim parent on a ref) are errors, never guesses. An id or a `reason` the driver does not know is kept as
`Other(..)`, **logged** and counted, never mapped to a known one.

| `refusal` | In plain words | What to do |
|---|---|---|
| `over-max-bytes` | the point holds more content than the bound | too large to compare here; raise the bound, or look at it elsewhere |
| `over-max-files` | the point has more files than the bound | the same |
| `destination-exists` | the destination is already there | cannot happen through the driver (it makes a fresh one) |
| `temporary-sibling-exists` | a half-made copy of an earlier run is next to the destination | cannot happen through the driver (fresh path) |
| `point-not-found` | the ref or block does not exist, or the ref has no published history yet | the point is gone or not yet received |
| `claim-not-found` | no such claim | the claim was never accepted here |
| `not-a-claim` | the id given as a claim is a block or something else | a caller's mistake |
| `claim-has-several-parents` | the claim names more than one parent (a merge) | cannot be previewed as one change |
| `case-collision` | two paths differ only by case | cannot be written on a case-insensitive disk; prikk refuses such a commit itself, so this needs crafted data |
| `no-checkpoint-in-reach` | no verified snapshot within 63 blocks (see `reason`) | see below |
| `damaged-repository` | prikk could not read what it needed | damaged **or**, for a claim, its parent is still pending (see below) |
| `io-failure` | a disk or resource failure while writing | an operator's problem |
| `not-a-repository` | there is no repository here | the caller bound the driver to the wrong directory |
| `claim-parent-not-sealed` | (prikk 0.55.0 and later) the claim's parent block is accepted here but not sealed yet | wait for the parent to be sealed; nothing is damaged (see below) |
| `destination-inside-repository` | (prikk 0.55.0 and later) the destination lies inside the repository | cannot happen through the driver; if it does, it is a **bug of the driver** (`DestinationInsideRepository`) |

`reason`, only for `no-checkpoint-in-reach`: **`signer-not-adopted`** — a recorded checkpoint is in reach but its signer is not
adopted here: adopt the key (the checkpoint-refresh job will not help). **`not-recorded`** — a snapshot is in reach but this
repository never recorded it: run `verify` (what the checkpoint-refresh job does). **`no-checkpoint`** — no block within 63
blocks has a snapshot: `verify` cannot help, and running it again will not clear it; only `--allow-full-replay` reads the point
(then `provisional: true`). A caller must not loop on this one.

**A claim whose parent is still pending.** In prikk 0.54.x `extract --claim` of a claim whose parent block is accepted but not
sealed here refuses with `damaged-repository` ("missing Block …"); the repository is not damaged. The driver does not say
"damaged" for a claim: it returns `ClaimParentNotSealedOrDamaged`, whose text names both possibilities (a test reproduces it with a
real accepted artifact of two commits: the first claim extracts, the second does not). **prikk 0.55.0 and later give it its own id**,
`claim-parent-not-sealed`, and the driver returns `ClaimParentNotSealed`: the repository is not damaged and the text says so. The
floor is 0.54.0, so both outcomes exist. `refusal-v1` has no field for the claim or its parent (only `command`, `refusal`,
`reason`, `message`); the two ids are in prikk's text, which the driver keeps **verbatim** and never parses (the claim is the one the
caller asked for). A chain of pending claims is prikk 0.56.0 work: until then the second claim of a series refuses this way.

`destination-inside-repository` (0.55.0 and later) is the refusal of a destination inside the repository. The driver only names a
destination in its own working directory, outside every repository, so it never asks for one; if prikk says it, it is
`DestinationInsideRepository`, a named driver bug, never an ordinary refusal. Both ids were provoked on a real 0.55.0 (the tests skip,
saying so, below 0.55.0).

## Bounds and cost

Defaults, in one place (`planeter_prikk::extract`): `--max-bytes` **64 MiB** of content and `--max-files` **10,000**, both
configurable by the caller. A point over either is refused by prikk before the first write, and is "too large to compare here".
`--allow-full-replay` is off by default (a full replay of a long history is unbounded work).

Memory (prikk 0.54.1, measured 2026-10-25, the data segment the memory ceiling limits): **twice the largest single file** plus
0.3 MiB for a tree, a single big file, a claim, a plan and a write alike (64 MiB file: 128 MiB; 1,000 files of 10 KiB: under 1 MiB);
0.7 MiB more per 1,000 pending patches; and **up to 8.6 ×** the file for a file **edited many times**, replayed from the start
(8 MiB edited 8 times: 7.0 ×; 63 times: 7.9 ×; 1 MiB or 16 MiB edited 63 times: 8.6 ×). `--plan-only` costs the **same memory** as
a write (0.015 s against 1.5 s for 1,000 files, but the same peak), so a plan does not make admission cheaper.

So the cost rule (`ReadVerb::Extract`) is **9.0 × the configured `--max-bytes` + 8 MiB** (floor 16 MiB), under the usual 1.5 ×
ceiling; the store size does not enter. At the default bound that is **584 MiB** of the heavy pool: a host with about 3 GiB of
usable memory or more admits it at the default budget; a smaller host lowers the bound. It over-reserves for the usual point
(a tree costs about 2 MiB) and cannot under-reserve for the worst measured.

## Not here

The page and the API; the "declared base disagrees" refusal (RFC 005 part 6); the checkpoint-refresh job and its schedule
(RFC 006 v2); any use of `extract` by an existing page.
