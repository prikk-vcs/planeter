# Maintainer keys: who a repository trusts, and what can be done about it (RFC 013, 0.9.0)

A repository **trusts** a set of maintainer keys. Every change in its history has to be signed by one of them, and `prikk verify` judges the **whole history** against the keys the repository trusts *now*. This page says what a trusted key is, where to read the list, what a maintainer can do about their keys and what nobody can do yet, and — for the person who runs the server — the one command that adds a key. The decisions are in `rfcs/accepted/013-maintainer-keys.md`; the facts about prikk are the ledger's PK-90 to PK-93 (`docs/src/planeter-prikk-dependency-ledger.md`). planeter **holds no key and signs nothing**; it only keeps the list prikk keeps.

## For a maintainer

### What you can read

The repository's home page and history page have a section **"Keys this repository trusts (N)"** under a native `<details>`. Every key is one list item: its id (`ed25519-` and 16 characters), its fingerprint in groups of eight, and **one sentence of where it came from**:

| the sentence | what it means |
|---|---|
| From the repository's creation. | The creator typed its fingerprint when the repository was made. |
| From the repository's creation (recorded on {date}). | The same, but planeter wrote the record on that date: the repository was made before planeter kept this record, and the key was found trusted then. (In every version before this one only creation could put a key there.) |
| Added by this server's operator on {date}: {reason} | The person who runs this server added it with `planeter operator key add`, and gave that reason. |
| Added by a delivery on {date}. | A maintainer's delivery added it. **Nothing writes this today** (see "What a delivery does not do"); the page can say it, so that the day it is built the pages do not change. |
| Source not recorded. | prikk lists the key, planeter has no record of how it came (it was added with prikk directly, outside planeter). **Nothing is claimed about it.** |

The list is prikk's own (`trust maintainer list`): it is the truth about what the repository trusts. planeter's record only says *how* each key came. The same list is in the JSON: `GET /api/v1/repos/{owner}/{name}/refs` has `trusted_keys` — each with `key_id`, `public_key`, `fingerprint` and a `source` (`kind` is `creation`, `delivery`, `operator` or `not_recorded`; the OpenAPI document describes each field). It is `null` when the list cannot be read now (prikk or the store did not answer): you are never shown a list from earlier as the current one. The page then says "The keys this repository trusts cannot be read now." The keys are public: a repository you may not see is a 404 and shows no keys.

The page never says a key is retired, revoked, expired, rotated or removed, because **planeter never removes a key** and no key can be marked as anything else (a test over the catalogs and the rendered pages holds the words).

### What you can do

* **Ask the operator to add a key.** Whoever runs the server adds it with the command below, with a reason that everyone who can see the repository will read. A key you have made with prikk is yours to give them: its id and its public key (`prikk trust maintainer list`).
* **Rotate by adding.** Have the new key added, sign new work with it, and **leave the old key trusted**. Both keys then verify; every delivery signed by either is accepted.
* **Keep several keys.** A repository can trust at most **64** keys (below). Two or three keys, kept in different places, are what protect you from losing one.
* **At creation,** adopt the keys your archive carries by typing their fingerprints (`docs/client.md`, `docs/deliveries.md`). The confirmation page says how many of the 64 slots creating from this archive leaves.

### What you cannot do

* **Remove a key.** No page, command or API of planeter does, and planeter never calls `prikk trust maintainer remove`.
* **Mark a key retired,** or make history it signed **stop counting**, or say that a key counts only up to a point.
* **Why:** prikk's `verify` judges the whole history against the keys trusted now. Measured on prikk 0.58.0 (PK-90): after a key is removed, `verify` **fails** for everything that key signed, whichever block it was. prikk confirms that this is what remove means — "this key's signatures never count" — and that **retiring a key from a point does not exist yet**; it is a question for prikk's trust design round (PK-91). Until it does, a forge that removed a key would break the repository's own checks.
* **Add a key by a delivery.** A delivery is accepted only if it is signed by keys the repository already trusts; it cannot bring in a key, and the keys an archive merely carries change nothing (`docs/deliveries.md`, "What a delivery does not do"). Adding a key through a delivery is not available.

### When a key is lost or stolen

* **A lost key:** leave it trusted (everything it signed still verifies) and have a new key added. Do not have it removed.
* **Every key lost:** nobody can sign a delivery. The operator's command is the way out: it adds a key without asking for a signature (prikk adopts a key without one: PK-91). Then seal new work with the new key. **Do not remove the lost keys:** the repository verifies only while they stay trusted.
* **A stolen key:** there is **no remedy that keeps the history verifiable**. Keeping the key trusted means what the thief seals also verifies; removing it means everything it signed fails `verify` (PK-91). So: **stop delivering from the other keys until you have decided, tell the people who rely on the repository, and wait for prikk's design.** A successor repository does not help: it cannot leave the key out, because the history it carries was signed by it and would not verify there either (an archive's unadopted key makes the receiver's `verify` fail: PK-91, case 4).

## For an operator

### The command

```
planeter operator key add OWNER/NAME --key-id ID --public-key HEX --reason TEXT [--lang en|ja]
```

**Stop the server first.** The command takes the server's own process lock (`{db}.lock`); with a server running it exits **3** and says "The server is running. Stop it, run this command again, then start it." It reads only `PLANETER_DB` and `PLANETER_REPOS_ROOT` (the server's own settings) and `PATH` (to find prikk), and the language from `--lang`, or `LC_ALL`, `LC_MESSAGES`, `LANG`; it never reads a secret of the server. Then start the server again.

* `--public-key` is the 64 lowercase hexadecimal characters of the key. `--key-id` is `ed25519-` and the **first 16 characters of the public key**: prikk 0.58.0 accepts any well-formed id for any key and would list a wrong one as a second entry for the same key (PK-93), so planeter derives the id itself and refuses one that does not belong to the key, showing the one that does.
* `--reason` is 1 to 200 characters on one line. **The reason is shown to everyone who can see the repository** (the page and the JSON), as text, never markup. A reason longer than 200 characters, empty, or with a control character such as a line break is refused (exit 2) and nothing is changed; other characters that disguise text (a bidirectional override, a zero-width character) are accepted and **shown as visible escapes** (`\u{202E}`), so what is read is what was typed. **If you record a key that was added earlier, say so in the reason:** the command cannot tell a key you are adding for the first time from one someone added to the repository with prikk directly, and the record it writes says what you say.

What it does, in order: the database must exist (it is never made); the lock; the repository must exist and be at rest (not flagged for recovery, no unfinished write or delivery); the key must not already be trusted and recorded; the repository must have room (below); the repository's write lease is taken (its durable record is the intent); `prikk trust maintainer add`; `prikk verify` plus the publication-trust check; the revision counter moves; the record of how the key came is written; the lease and the lock are released. A repository that had no record of its keys yet is given the record of the keys it already trusts (source "creation", recorded later) **before** the add, so the one new row never makes it look recorded when it is not.

### Exit codes and what is said

| code | meaning |
|---|---|
| 0 | done: "{fingerprint} is now trusted by {repo}. Added by this server's operator on {date}: {reason}" and "Start the server." |
| 1 | refused, and **nothing was changed**: the repository does not exist; "Start the server and let recovery finish first." (it is flagged, or a write or a delivery is unfinished); "This key is already trusted."; the key id does not belong to the key; the bound |
| 2 | the command line is not understood, the reason is missing, too long or has a character that cannot be shown, a key or an id is malformed |
| 3 | the server is running, or its lock cannot be taken |
| 4 | **something was changed and not finished** (below) |
| 5 | local files or the database (no database at `PLANETER_DB`; it cannot be opened) |

Exit **4** has two forms. "The key was added, but the repository no longer passes its checks, and a key cannot be taken out again": the repository is marked as needing recovery; start the server and ask the repository's administrator to look at it. Or "The key is trusted but the record of it was not written. Run the same command again to record it." A run killed at any point is picked up the same way: the server's next start finds the unfinished add, marks the repository, and recovers it when every witness agrees; running the same command again then completes it (if the key is already trusted and the record is missing, it writes the record and nothing else).

### The bound: 64 keys

A repository trusts at most **64** keys, by every path (creation, this command). Keys cannot be removed yet, so each key added uses a slot for the repository's whole life, and a repository at 64 refuses another with "This repository already trusts the most keys planeter supports: 64. Keys cannot be removed yet, so no more can be added." The number is a measurement (PK-92, prikk 0.58.0): with 17, 65, 257 and 1,025 trusted keys `verify` took 7, 10, 29 and 553 ms, and the archive of the repository grew with the square of the number of adds (70 KB at 65 keys, 13.8 MB at 1,025). At 64 nothing needs compacting; if the bound is ever raised, `prikk compact --trust-policy` after each add is part of that change.

The keys of each repository, and how each came, are also readable in the JSON of `refs` (above). The creation confirmation page says how many slots a creation leaves; `keys_max` is in the stage's JSON.
