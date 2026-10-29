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
repository). `304` means nothing changed. **Freshness (RFC 004 D-8):** the program records, per server and repository, the tip of each
published branch it was shown, in the state directory; on the next fetch the new archive is first landed in a throwaway repository, and
if a recorded tip is not part of the history the server now shows for that branch, the fetch stops with the split-view sentence and
**nothing reaches your repository**. The advertisement carries no `update_seq` yet, so an older state cannot be told from a newer one by
number: the program says so on every fetch and compares what each branch continues from instead.

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
on `PATH`; the tests use it for a stand-in). prikk finds your keys by its own rules (your home directory); the program never reads them.

## Exit codes

`0` done · `1` the server refused (or a state of yours it cannot work with) · `2` the command line is wrong · `3` the network or TLS failed
before an answer · `4` your prikk failed or is missing or too old · `5` local files, the state directory, the token file.

## Colour, terminals, languages

The output is plain lines. There is no colour (so `NO_COLOR` changes nothing), no spinner and no in-place redraw; progress is an
occasional whole line. The language is `--lang`, else the first of `LC_ALL`, `LC_MESSAGES`, `LANG`; **English** unless a shipped
language is named. **Japanese is a draft** for the owner's and a colleague's review (RFC 012 D-3f): it is used only with `--lang ja`
until `JA_REVIEWED` is set. The messages are in `src/catalog/`; the plain-words rules (no *seal*, *adopt* or *intent* in a sentence) and
the agreement of the two tables are tests.

## Building

`cargo build -p planeter-client --release --locked`. The program is built and tested on **Linux (`ubuntu-24.04`), macOS (`macos-15`) and
Windows (`windows-2025`)** in CI (the `client` job; pinned runner labels, no `-latest`). **Build requirements, measured** (the CI job builds
a second time with `nasm` and `cmake` made unusable): on all three systems the build needs **a C compiler** (aws-lc's C sources are compiled
by the `cc` crate: the system compiler on Linux and macOS, the MSVC build tools on Windows) and **nothing else** — `nasm`, `cmake`, `perl`
and `go` are not needed. The runners carry them anyway; a clean checkout on a machine with only a compiler builds. Tested: Windows x86_64
(MSVC); other targets were not tried. The binary is separate from the server's (`planeter`), and the server's crates do not depend on its
HTTP and TLS stack (a test in `tools/boundary-check` keeps it so).

**What CI runs.** The program is built, and its HTTP/TLS core, command line and messages are tested, on Linux, macOS and Windows. The three
commands themselves are tested on **Linux** only (they need prikk and, for the end-to-end test, the server's sandbox).

## Not here

Distribution (release assets, `cargo install planeter-client`), SSH, a `merge`/`seal`/`publish` command, signing of any kind, auto-update
and telemetry are not part of this program.
