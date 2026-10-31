# planeter-client: your own prikk, talking to a planeter server

| | |
|---|---|
| Document | What `planeter-client` does, what it never does, how it trusts, its exit codes and how to read its TLS errors (RFC 004 part 9, D-10, D-50). |
| Where | `crates/planeter-client` (the program), `crates/planeter-client/tests/` (its tests: the HTTP core, the command line, the messages, the commands against a stub server, and `crates/planeter/tests/client_e2e.rs` against the real server behind a TLS terminator). |
| Needs | prikk 0.54.0 or later on your `PATH` (your own prikk and your own keys); a planeter server reachable over **https**. |

`planeter-client` does three things with a repository on a planeter server, and nothing else:

* **`clone`** downloads it, checks it with your prikk, shows you the maintainer keys it carries, and trusts **only the keys you type**.
* **`fetch`** asks whether the server's state changed; if it did, downloads it, checks it, and lands it as **received copies**. Your own branches are never touched.
* **`propose`** builds your work against what the server already holds, with your prikk and your keys, and sends it as a **proposal**. The server holds it for review; no branch changes.

It holds **no key**, links no prikk code, and signs nothing: every history operation is your own prikk. There is no `merge`, `seal`
or `publish` command, no SSH, no auto-update and no telemetry.

## Trust, in four sentences

A repository carries the public keys of the maintainers who signed its history; the server cannot vouch for them and neither can this
program. `clone` shows each key in full, in groups of eight characters, and trusts a key only when you type it (spaces optional) or
name it with `--trust-key`; pressing Enter skips a key, and a skipped key trusts nobody. With no key trusted, or with `--no-adopt`, the
repository lands as received copies and **nothing is verified or trusted**. Trust never comes from the server, a default, or a
"yes" answer.

## The commands

```
planeter-client propose <https://host/owner/name> [--ref heads/NAME] [--proposal ID] [--title TITLE]
                        [--description-file FILE] [--dry-run] [--retry]
planeter-client clone   <https://host/owner/name> [DIR] [--trust-key KEY]... [--no-adopt]
planeter-client fetch   [DIR]
every command:          [--token-file FILE] [--cacert FILE] [--lang en|ja] [--verbose]
```

Put the command first. `--help` after a command shows that command's options and one example; `--version` prints the name, the
version, the build commit and the prikk floor, and needs no network.

**`propose`** runs inside your prikk repository. It asks the server for its advertisement and for the **have-list** of the branch
(`GET …/transport/have?ref=…`, RFC 004 part 8: only a published branch has one), runs `prikk sync build <ref> --have FILE`, declares
the patch ids of the blocks above the server's tip, and sends the artifact in the propose framing. It then says in plain words what
happened and what comes next:

> Proposed as proposal #c-42 (revision 1). Your work is held for review on this server; no branch has changed. Reviewers can see it at
> https://forge.example/alice/app/proposals/c-42.

`--dry-run` builds and declares, says what would be sent, and sends nothing (it needs no token). A refusal is **one sentence for the
reason and one for what to do**; the server's own words (`code`, `message`, `hint`) appear only under `--verbose`. If the server did
not finish taking the work in (`incomplete`), the same work is sent again only with `--retry`: the program never repeats a send by
itself, and the record of an unfinished send is the only thing it keeps about a proposal.

**`clone`** needs the keys typed at a terminal, or `--trust-key` for each key you trust when there is none (a pipe, a script). With
neither, it stops before downloading anything and says how. A `--trust-key` the repository does not carry stops the clone: nothing is
put in your repository.

**`fetch`** knows the server from the clone it made (the address is recorded in this program's state directory, never in the
repository). `304` means nothing changed. **Freshness (RFC 004 D-8):** the program records, per server and repository, for each published
branch it was shown, the tip, the RefState id and the **`update_seq`** (the advertisement carries it), in the state directory; the number
is only ever raised. On the next fetch the advertisement is compared with the record **before anything is downloaded**: a *lower*
`update_seq`, or the *same* number with another RefState id, stops the fetch at once (the server is going back in time, or two servers
answer under one name). A *higher* number is downloaded, landed in a throwaway repository, and the recorded tip must be part of the
history it shows for that branch, else the fetch stops with the split-view sentence; either way **nothing reaches your repository**. The
same number with the same state is the state already seen. A server whose advertisement has no `update_seq` (an older planeter) falls back
to the continuation check alone, and the program says so; that sentence appears only then.

**`propose` and the server's bounds.** The advertisement also says what a proposal may carry (`limits.max_artifact_bytes` and
`limits.max_objects`, absent when the server takes no proposals). The program refuses work over either bound **before uploading anything**,
with the server's own number in the sentence and the remedy (send smaller pieces, or ask the person who runs the server to raise the
limit): the patch count before the build, the artifact's size after it. Against a server that states no bound it sends, and the server's
`413` — answered before a byte of the body is read — is the check.

## The token

From `PLANETER_TOKEN`, or from `--token-file FILE`; **never from the command line** (a token on a command line is in the process list
and the shell history). The file must not be readable by group or others on Unix (`chmod 600`), else it is not used and the sentence says
so. **Windows** has no permission bits to check: the program does not look at the file's access control list; keep the token file inside
your user profile, whose access list is private by default. The token is added to a request in one place, is in no message, log or
`--verbose` line, and a test greps every output of every run for it. Reading (`clone`, `fetch`) needs no token on a public repository.

## https, certificates, proxies

* **https only.** A plain `http://` address is refused before any connection. There is **no `--insecure`**.
* **Redirects are reported, never followed**, and cookies do not exist.
* The trust anchors are the **Mozilla root list compiled into the program** (`webpki-roots`); `--cacert FILE` **replaces** that list with
  the certificates in the file (it does not add to it). A server with its own certificate authority therefore needs `--cacert`.
* `HTTPS_PROXY`, `NO_PROXY` and their lower-case forms are honoured. A proxy that needs a login is not supported.
* Timeouts: 10 s to connect, 60 s to the first byte, 30 s between reads, four hours in all; a download that is cut keeps its
  `.partial` file and resumes with `Range` and `If-Range` on the next run. Nothing is held in memory except small JSON answers (read up
  to 32 MiB) and an error body (read up to 64 KiB).

### Reading a TLS error

| You see | It means | Do |
|---|---|---|
| "…not signed by a certificate authority this program trusts…" | the server's certificate chain ends at an authority that is not in the built-in list (a private authority, or an interception proxy) | pass `--cacert FILE` with that authority's certificate, if you know the server uses its own |
| "…certificate is for another name than the one in the address…" | the certificate does not list the host you typed (an IP address typed for a name-only certificate, a wrong host) | use the name the certificate is for |
| "…has expired (or is not valid yet)…" | the certificate's dates exclude now | tell the person who runs the server; check your computer's clock |
| "The server at … could not be reached" | refused, unknown host, no route | check the address and your connection |
| "The secure connection … could not be made" | any other TLS failure | run again with `--verbose` for rustls' own words |

## Environment

`PLANETER_TOKEN` (the token); `HTTPS_PROXY`, `NO_PROXY` and their lower-case forms; `LC_ALL`, `LC_MESSAGES`, `LANG` (the language);
`PLANETER_CLIENT_STATE_DIR` (where the program keeps its two small records — the tips it was shown, and which directory came from which
server — default: the platform's usual state directory); `PLANETER_CLIENT_PRIKK` (the path of the prikk program to run instead of the one
on `PATH`). **Why it exists:** a test runs a stand-in prikk through it, and a person with prikk in an unusual place can name it. **What it
is not:** a way to choose another version-control tool (the program speaks prikk's commands and reads prikk's reports), and not a secret —
whatever it names runs with your own rights, exactly as the `prikk` on your `PATH` would. It is deliberately not in `--help`. prikk finds
your keys by its own rules (your home directory); the program never reads them: it passes `HOME` and `XDG_CONFIG_HOME` on to the prikk it
starts, unchanged (the tests give each person their own).

**When prikk will not start.** "prikk was not found" is said only when there is no such program. A prikk that is there and cannot be started
is told as that, with the system's own reason (`Permission denied`, `Text file busy`, a script whose interpreter is gone) and the next step
(run `prikk --version` by itself); a prikk that starts but does not say a version is told what it said instead.

## Exit codes

`0` done · `1` the server refused (or a state of yours it cannot work with) · `2` the command line is wrong · `3` the network or TLS failed
before an answer · `4` your prikk failed or is missing or too old · `5` local files, the state directory, the token file.

## Colour, terminals, languages

The output is plain lines. There is no colour (so `NO_COLOR` changes nothing), no spinner and no in-place redraw; progress is an
occasional whole line. The language is `--lang`, else the first of `LC_ALL`, `LC_MESSAGES`, `LANG`; **English** unless a shipped
language is named. **Japanese** (signed off on 2026-10-31, RFC 012 D-3f; `JA_REVIEWED`) is chosen when one of those variables says `ja`
(`ja_JP.UTF-8`, `ja`), and with `--lang ja`; `--lang` always wins over the environment. The messages are in `src/catalog/`; the plain-words rules (no *seal*, *adopt* or *intent* in a sentence) and
the agreement of the two tables are tests.

## Building

`cargo build -p planeter-client --release --locked`. The program is built and tested on **Linux (`ubuntu-24.04`), macOS (`macos-15`) and
Windows (`windows-2025`)** in CI (the `client` job; pinned runner labels, no `-latest`). **Build requirements, measured** (the CI job builds
a second time with `nasm` and `cmake` made unusable): on all three systems the build needs **a C compiler** (aws-lc's C sources are compiled
by the `cc` crate: the system compiler on Linux and macOS, the MSVC build tools on Windows) and **nothing else** — `nasm`, `cmake`, `perl`
and `go` are not needed. The runners carry them anyway; a clean checkout on a machine with only a compiler builds. Tested: Windows x86_64
(MSVC); other targets were not tried. The binary is separate from the server's (`planeter`), and the server's crates do not depend on its
HTTP and TLS stack (a test in `tools/boundary-check` keeps it so).

**What CI covers.** The `client` job builds the program and runs `cargo test -p planeter-client` on `ubuntu-24.04`, `macos-15` and
`windows-2025`, after installing prikk 0.54.0 (the floor) from the project's release assets, pinned by version and checked against the release's
own `.sha256` (prikk publishes Linux, macOS arm64 and Windows x86_64 builds). Which tests run where:

| Tests | Linux | macOS | Windows |
|---|---|---|---|
| the HTTP/TLS core, the command line, the messages, the freshness comparison, the floor, `--version` (`http`, `cli`, `catalog`, `freshness`, `floor`, `version`) | yes | yes | yes |
| the prikk runner against the real prikk (`prikk_runner`) | yes | yes | yes |
| `clone`, `fetch`, `propose` against a stub server with the real prikk (`flows`), and the proxy variables (`proxy`) | yes | yes | **no** — they give each person their own `HOME` for prikk's keys and use POSIX file modes; prikk's key location on Windows differs, and a test must not write to a developer's real keys |
| a prikk that is there but cannot be started, told as that and not as "not found" (`prikk_start`) | yes | yes | no (modes and scripts) |
| the real planeter server behind TLS, with the real client (`crates/planeter/tests/client_e2e.rs`, in the `build` job) | where the server's sandbox works (Linux) | no (the server is Linux only) | no |

So the three commands are exercised on Linux and macOS; on Windows the program is built and its stack, command line, messages and prikk
runner are tested.

**Tests that must not flake.** The `flows` suite starts the real prikk from about twenty threads at once, against stub servers in the same
process. Two things were found to fail only under load, and are why the suite is written as it is: a test never writes an executable (a
script open for writing in a thread's forked child cannot be started, `Text file busy`, so the tests give each person their own `HOME`
through the context and run the one real prikk), and the stub server closes a connection gracefully (shuts down its writing side and reads
what the client still sends before it closes; a socket closed with unread data answers with RST, which can throw away the response the
client had not yet read). To check that it stays so, run the stress target by hand; it is ignored by default:

```text
cargo test -p planeter-client --test stress -- --ignored --nocapture
PLANETER_STRESS_RUNS=50 PLANETER_STRESS_CPUS=0,1 cargo test -p planeter-client --test stress -- --ignored --nocapture
```

It runs the `flows` suite `PLANETER_STRESS_RUNS` times (default 10), one run after the other, and fails on the first run that fails, naming
the failing tests. With `PLANETER_STRESS_CPUS` (Linux, `taskset`) every run is confined to those CPUs and one busy loop per CPU is started
on them; the busy loops are children of the test and are stopped through their handles. Run it inside a memory-limited scope
(`systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 …`). There is no scheduled CI job for it.

## Not here

Release-asset binaries of the client (0.5.0 ships it through crates.io, `cargo install planeter-client --locked`, and as source), SSH, a `merge`/`seal`/`publish` command, signing of any kind, auto-update
and telemetry are not part of this program.
