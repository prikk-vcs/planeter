# RFC 003 T7 — Read-path gap catalogue (D-7 / UD-1)

Where a browse view needs repository data that prikk does not (yet) expose in machine-readable
(`--format json`) form. Per `LAY-3`, planeter **never reaches into prikk internals** to fill these — the
view renders **"pending a prikk increment"** and the gap is listed here as the prikk-side ask.

Verified against prikk **0.43.0**'s read surface (`log`, `show`, `verify`, `branch`, `tag`, `status`,
`worktree-status`, `checkout --patch-plan --content-path`).

## Gaps (candidate prikk asks)

1. **Directory/tree listing at a ref.** `checkout --patch-plan --content-path` returns content for
   **explicitly named** paths; there is no verb that enumerates the paths (a file tree) present at a ref.
   Directory browsing — the spine of a forge's file view — needs one. *Ask:* a `--format json` tree/list
   of paths (and modes) at a ref, paginated.
2. **Raw blob byte retrieval by id.** `show`/`checkout` report binary content as `{ blob_id, size }`
   (metadata), and text inline; there is no verb to stream the **raw bytes** of a blob by id. The raw
   file view, downloads, and hosted-content serving (from the isolated origin, T4) need it. *Ask:* a verb
   that emits a blob's bytes by id (streamable, size-bounded).
3. **Blame / annotate.** No machine-readable per-line attribution (which patch last touched each line).
   A standard forge view; not derivable from `show`/`log` without reaching into internals. *Ask:* a
   `--format json` blame for a path at a ref (may be a later increment).
4. **Diff between two arbitrary refs/blocks.** `show` yields one patch/block's operations; there is no
   verb for a general "diff ref A vs ref B" effect. Compare views need it. *Ask:* a `--format json`
   effect-diff between two named refs/blocks.

## Deliberate non-gaps (do not ask)

- **Timestamps / commit dates.** prikk has **no clocks** by design (no HEAD, no wall-clock in history).
  The UI orders by `update_seq` and block lineage, and shows **no fabricated dates** — this is honesty
  (`SEC-1`/`WEB-06`), not a gap. Never synthesize a timestamp.
- **A single "current commit" pointer.** prikk has no HEAD; `branch`/`status` report ref state. The UI
  reflects refs, not a HEAD.

## Handling until closed

Each gapped view ships now with an explicit **"pending a prikk increment"** state (never a blank or a
faked value), and the read API returns a documented `501`-style "not yet available" for the endpoint.
When RFC 003's review-request package is assembled, these become the letter to the prikk team (the same
channel as the RFC 154/155 asks), prioritized: **(1) tree listing** and **(2) raw blob bytes** are M1
browse blockers for a familiar forge; **(3) blame** and **(4) compare** are M2+.
