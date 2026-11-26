# Accounts: who has one, how they sign in, and what the person who runs the server can do about it (RFC 014, step 1)

An **account** is a person's place on a planeter server. This page says what an account is, what the person who runs the server (the **operator**) can do with accounts and tokens today, how a sign-in provider fits in, and, for a person who has an account, how to get a token and what it lets them do. It is written for two readers, in this order: the operator, then the person with an account. The decisions are in `rfcs/accepted/014-accounts-and-sign-in.md` (and its Amendment 1); what the sign-in provider told us is in `docs/src/planeter-idp-ledger.md` (rows ID-1 to ID-10); the facts about maintainer keys are in [`keys.md`](keys.md). Everything below is what is built. What is not built is in one paragraph at the end ("What is not here yet"), and nothing else on this page promises it.

**Every sentence in quotation marks or in a table below is the program's own sentence**, copied from its English message catalog and checked word for word by a test (`crates/planeter-web/tests/accounts_doc.rs`), so that this page cannot drift from what the commands print. A `{word}` in braces is filled in by the program when it speaks (`{handle}` is the account's handle, `{repo}` is `OWNER/NAME`, and so on).

planeter is a **relying party**: it asks a sign-in provider who a person is, and it is not one. It has no second factor, no email, no "forgot password" page, no password reset by mail and no account recovery flow. A person who cannot sign in is helped by the provider (when they sign in through it) or by the operator (when they have a local password).

## What an account is

* **A handle.** The account's identity and its namespace (`/alice/forge`). Lowercase ASCII letters, digits, `-`, `_` and `.`; it starts with a letter or digit; 1 to 39 characters; no `..` and no `.` at the end; not one of the words the server uses for its own pages; unique **without regard to case**, among accounts, organizations and repository owners together. **A handle is never changed and never reused**, even after the account is disabled (a handle someone else could take over would let them be mistaken for the person who had it).
* **A display name.** Text, 1 to 100 characters, shown beside the handle. It cannot hold a character that cannot be shown, such as a line break.
* **How it signs in.** A **local password**, a **linked identity at the sign-in provider**, or both. An account made without a password signs in only through a linked identity.
* **A state.** Active, or disabled (below).

**No email address is used to find, link or merge anyone.** An identity at the provider is linked to an account by the pair *(issuer, subject)*: the provider's address and the provider's own stable identifier for the person, and by nothing else. (An address a person can claim is exactly how accounts get taken over.)

Handles made before this rule existed keep working as they are. Two older handles that differ only by case (`Alice` and `alice`) are **reported, never renamed**: `planeter operator account list --check` lists them (below).

## For the person who runs the server

### How the commands are run

```
planeter operator account create|set-password|disable|enable|link-oidc|unlink-oidc|grant|revoke|list|show …
planeter operator token create|list|revoke …
```

Every command also takes `--lang en|ja` (the language of what it says; without it, the one `LC_ALL`, `LC_MESSAGES` or `LANG` names, else English).

* **They run beside a running server.** An account is a row of the server's database, not a repository, so **you do not stop the server** (`planeter operator key add` is different: it does, [`keys.md`](keys.md)). The server looks at accounts, sign-ins and tokens at every request, so what a command does is seen on the very next one. If the database stays busy for more than 5 seconds because another process is writing, the command says so and changes nothing; run it again.
* **They read `PLANETER_DB`** (the server's database, which must exist: start the server once first) and, for `link-oidc` and `unlink-oidc`, **`PLANETER_OIDC_ISSUER`**, to check that the identity names the provider this server is set up for. They read no other setting, and never the provider's client secret. Set them as the server's own settings are set.
* **A refusal changes nothing.** Each command is one step (one database transaction), so a change is never left half done.
* **Each success says what it did in the sentence the account's holder will see in their own log** (the account log, below), and then one line of what it means.
* **Exit codes.** `0` done. `1` refused, and nothing was changed (the account or the repository does not exist, the account is in the wrong state, a rule of the command). `2` the command line is not understood, a value is malformed, or no terminal is there to type a password on. `5` local files or the database (no database at `PLANETER_DB`; it cannot be opened; it stayed busy). **`3` and `4` are not used** (they are `key add`'s): `3` means "the server is running", and these commands do not need the server stopped; `4` means "something was changed and not finished", and each act here is one step, so a change is never left half done.

The help of each family is `planeter operator account --help` and `planeter operator token --help`. A command line that is not understood says "This command line is not understood. Run planeter operator account --help to see what it takes." (exit 2), and, for a token command, "This command line is not understood. Run planeter operator token --help to see what it takes."

### The commands, with an example of each

The examples were run against a scratch database; an output line that depends on the account or the day (a date, an id) will differ in yours.

**`account create HANDLE --name TEXT [--password-prompt | --password-fd N]`** makes an account. Without a password option the account has no password and signs in only through a linked identity.

```
$ planeter operator account create alice --name "Alice Example"
The account was created by this server's operator.
alice (Alice Example) has no password: it can sign in only through a linked identity.
```

With a password option the account is made with a password (see "Passwords", below); the line then says "{handle} ({name}) has a password."

**`account set-password HANDLE [--password-fd N]`** sets the account's password (it can be the first). It signs the account out everywhere; its tokens are not touched.

```
$ planeter operator account set-password alice --password-fd 3 3<password-file
A password was set by this server's operator.
It was signed out everywhere; its tokens are not touched.
```

**`account disable HANDLE --reason TEXT`** and **`account enable HANDLE`** (below, "Disable, never delete").

```
$ planeter operator account disable bob --reason "left the team"
The account was disabled by this server's operator: left the team
It was signed out everywhere at once and its tokens are refused. Nothing it owns was deleted, and others can still read its repositories.

$ planeter operator account enable bob
The account was enabled by this server's operator.
Its tokens work again until they expire. The sign-ins that ended do not return.
```

**`account link-oidc HANDLE --issuer URL --subject TEXT`** links the account to one identity at the provider this server is set up for; **`account unlink-oidc`** takes the same words and removes the link. The issuer must be the value of `PLANETER_OIDC_ISSUER`. An identity is linked to at most one account, and is never moved from another (unlink it there first).

```
$ planeter operator account link-oidc alice --issuer https://idp.example --subject 3f0c…
An identity at idp.example was linked to the account by this server's operator.
alice can now sign in through that provider.
```

**`account grant OWNER/NAME HANDLE --role read|write|maintain|admin`** gives the account exactly one role on a repository, and prints the display name it granted to, so that a mistyped handle is seen at once. **`account revoke OWNER/NAME HANDLE [--allow-no-admin]`** takes away the role the account has **of its own** on the repository. A repository's owner stays its administrator whatever grants are taken away, and the owner and an organization's administrators count when the "last administrator" is judged: the last one stays unless `--allow-no-admin` is given.

```
$ planeter operator account grant alice/forge bob --role read
The role "read" on alice/forge was given by this server's operator.
Bob B (bob) was given read on alice/forge: look at the repository and clone it
```

**`account list [--check]`** prints the accounts sorted by handle, never a hash or a secret. With `--check` it prints, instead, the handles that differ only by case.

```
$ planeter operator account list
HANDLE | NAME | STATE | PASSWORD | IDENTITY | CREATED
alice | Alice Example | active | yes | no | 2026-11-26
bob | Bob Builder | active | no | no | 2026-11-26

$ planeter operator account list --check
No handles differ only by case.
```

**`account show HANDLE`** prints one account: its state, its identities, the roles it has of its own and **its last 50 events**.

```
$ planeter operator account show alice
Handle: alice
Name: Alice Example
State: active
Created: 2026-11-26
Password: yes, set on 2026-11-26
Identities linked: 0
Roles: none of its own
Last events:
  2026-11-26  A password was set by this server's operator.
  2026-11-26  The account was created by this server's operator.
```

**`token create HANDLE --name TEXT --scope read-only|write|full [--expires-days N] [--repos OWNER/NAME[,OWNER/NAME…]] [--out FILE]`**, **`token list HANDLE`** and **`token revoke HANDLE ID`**: tokens, below.

### Passwords

* **A password is never an argument and never an environment variable**, so it is never in the process list, the shell history or an environment dump. It is typed at the terminal **without echo, twice** (`--password-prompt` of `create`, and always for `set-password` without `--password-fd`), or read as **one line from an open file descriptor** for a script (`--password-fd N`: the script opens the file or the pipe on that number first). If a terminal stops echoing after an interrupted command, run `stty sane`.
* **The rule.** At least **12** and at most **128** characters, counted after the password is normalized (Unicode NFKC, so that the same password typed through two input methods is the same password); **not equal to the handle**; **not one of the most common passwords** (a list kept inside the program: nothing is fetched from anywhere). There are no rules about capitals or symbols (they teach people worse passwords). Every refusal says which rule failed, and nothing is changed.
* **Stored** only as an Argon2id hash. A password never appears in a log, an event or a message.
* **Why there is no reset by email.** planeter keeps and uses no email address, and a reset by mail would make the mailbox the real password. An operator who is asked for a new password runs `set-password`, which signs the account out everywhere. A person who signs in through the provider is helped by the provider.
* **A local password account has no second factor.** planeter does not build one: a second factor is the provider's job, so an operator who wants one has the accounts sign in through a provider that has it.

### Tokens

A token is what `planeter-client` presents instead of a password. The operator makes it for the account; the account's holder uses it.

* **Shown once.** By default the token is printed on **one line of standard output and nothing else goes there**, so that `TOKEN=$(planeter operator token create …)` works; what it is and when it ends go to standard error. With `--out FILE` it is written to a **new file made exclusively, with mode 0600** (an existing file is never overwritten), and not printed. Only its hash is kept: it cannot be shown again, and nobody can read it back, the operator included. Give a person the file or the line, and have them keep it private.
* **Scope, in words.** `read-only`: look at repositories and clone them, and nothing else. `write`: also propose work, but **not deliver a branch**. `full`: everything the account can do, including delivering a branch. A token never has more than the account has: its reach is the intersection of its scope and the account's own roles.
* **Expiry.** 1 to 365 days (`--expires-days`, default 90). **There is no token without an end.**
* **Repositories.** `--repos` limits the token to at most 20 repositories. Without it the token reaches every repository the account can.
* **Name.** 1 to 60 characters, to tell the account's tokens apart; a character that cannot be shown (a line break) is refused.
* **At most 200 tokens that have not expired** per account.
* **Revoke and disable refuse a token at the very next request.** `token revoke HANDLE ID` refuses that token from then on. `account disable` refuses every token the account holds (and `account enable` makes them work again until they expire).
* **Recorded as the operator's.** A token made by this command is in the account's log as made by "this server's operator", so that its holder sees who made it. Tokens made before 0.10.0 are named "(created before 0.10.0)" and keep working.
* **A token cannot make or revoke tokens**, and the commands above are the only way to make one today.

```
$ planeter operator token create alice --name laptop --scope write --expires-days 30
plt_…                                    (standard output: the token, on one line)
Access token "laptop" (write, all repositories the account can reach, until 2026-12-26) was created by this server's operator.
A token for alice (laptop) was created. Its id is 698cceeba173d9b8.
It can read and propose work, but it cannot deliver a branch.
It reaches every repository the account can reach.
It ends on 2026-12-26.
This is the only time the token is shown. Keep it somewhere private: it cannot be shown again.
                                         (the last six lines: standard error)

$ planeter operator token create alice --name ci --scope read-only --repos alice/forge --out ci-token
The token was written to ci-token (mode 0600). Give that path to planeter-client --token-file.

$ planeter operator token list alice
ID | NAME | SCOPE | REPOSITORIES | CREATED | LAST USED | EXPIRES
698cceeba173d9b8 | laptop | write | all | 2026-11-26 | never | 2026-12-26

$ planeter operator token revoke alice 698cceeba173d9b8
Access token "laptop" was revoked by this server's operator.
Token 698cceeba173d9b8 (laptop) of alice was revoked. It is refused from the next request.
```

### Disable, never delete

There is **no way to delete an account**, on purpose: deleting would orphan its repositories' namespace, its proposals and comments and its log, and would free its handle for someone else to take. **Disable** instead.

* `account disable HANDLE --reason TEXT` (1 to 200 characters, shown to the account's holder in their log and to the operator, **not to other people**) stops the account **at once**: it cannot sign in, it is **signed out everywhere**, and **every token it holds is refused**, each checked at the next request. A password sign-in on a disabled account answers exactly as a wrong password does (nothing tells a stranger which handles exist). A person who has just proved who they are **through the provider** is told plainly: "This account has been disabled. Ask the person who runs this server."
* **Nothing it owns is deleted**: its repositories, proposals and comments stay, and show its handle; others can still read its repositories.
* **Repositories can be left without an administrator.** `disable` first names the repositories for which this account is the only administrator, and still goes ahead: "These repositories will have no administrator until you grant one: {repos}" (and, when the list is long, "… and {more} more"). Grant an administrator to each. `revoke` refuses to take the last administrator away unless `--allow-no-admin` is given.
* `account enable HANDLE` undoes it: the tokens work again until they expire; **the sign-ins that ended do not come back**, the person signs in again.
* A disabled account cannot be given a role or a token: enable it first.

### Roles

A role is what an account may do on one repository. One plain sentence each (these are the words `grant` prints):

* **read**: look at the repository and clone it.
* **write**: also propose work.
* **maintain**: also ask for a delivery and review proposals.
* **admin**: also manage who has access.

**A role and a maintainer's signing key are different things.** Granting `maintain` lets a person ask for a delivery; it does not make their key trusted. Keys are the repository's, and the operator adds them ([`keys.md`](keys.md)).

### The account log

Every act of the commands above leaves an event in **the account's own log**, which its holder is meant to be able to read: an account created; a password set (and the sign-ins that ended with it); an identity linked or unlinked; an access token created or revoked; a role given or taken away; the account disabled (with the reason) or enabled. Each event says **who** did it, in these words: "you" (the account's holder), "this server's operator", or "the sign-in provider".

**A log never holds a secret, a token, a hash or an address.** An event keeps data (a kind, a time, who, and a few facts: a role, a scope, a count, a repository), and the program writes the sentence from it in the reader's language. Today the log is read by the operator, with `account show` (the last 50 events); the page where a holder reads their own is not built (below).

The sentences, with the plural forms the language uses:

| the event | the sentence |
|---|---|
| created | The account was created by {by}. |
| password set | A password was set by {by}. |
| password changed by its holder (**nothing writes this one yet**: it is for the page where a person changes their own password, step 2) | The password was changed by {by}. |
| sign-ins ended, said after another sentence (setting a password signs the account out) | {count} sign-in ended. ‖ {count} sign-ins ended. |
| sign-ins ended by someone (**nothing writes this one yet**: it is for the page where a person ends their own sign-ins, step 2) | {count} sign-in was ended by {by}. ‖ {count} sign-ins were ended by {by}. |
| identity linked | An identity at {host} was linked to the account by {by}. |
| identity unlinked | An identity at {host} was unlinked from the account by {by}. |
| token created | Access token "{name}" ({scope}, {repositories}, until {date}) was created by {by}. |
| token revoked | Access token "{name}" was revoked by {by}. |
| role given | The role "{role}" on {repository} was given by {by}. |
| role taken away | The role "{role}" on {repository} was taken away by {by}. |
| disabled | The account was disabled by {by}: {reason} |
| enabled | The account was enabled by {by}. |
| a sign-in by a method the account had not used before (**nothing writes this one yet**) | The account signed in by a method it had not used before. |

When a stored fact cannot be read, the program says the plain form of the sentence and never shows raw stored text:

| the event | the plain sentence |
|---|---|
| identity linked | An identity was linked to the account by {by}. |
| identity unlinked | An identity was unlinked from the account by {by}. |
| token created | An access token was created by {by}. |
| token revoked | An access token was revoked by {by}. |
| role given | A role on a repository was given by {by}. |
| role taken away | A role on a repository was taken away by {by}. |
| disabled | The account was disabled by {by}. |

and the short names inside them: the repositories of a token are "{count} repository ‖ {count} repositories", or "all repositories the account can reach"; the roles are "read", "write", "maintain" and "admin" (the words of `--role`); {by} is "you", "this server's operator" or "the sign-in provider".

### What each command says when it worked

Each of these is printed after the sentence of the event (the account log's sentence, above). `{means}` is the one plain sentence of the role (above).

| the sentence | after |
|---|---|
| {handle} ({name}) has a password. | `create` with a password |
| {handle} ({name}) has no password: it can sign in only through a linked identity. | `create` without one |
| It was signed out everywhere; its tokens are not touched. | `set-password` |
| It was signed out everywhere at once and its tokens are refused. Nothing it owns was deleted, and others can still read its repositories. | `disable` |
| Its tokens work again until they expire. The sign-ins that ended do not return. | `enable` |
| {handle} can now sign in through that provider. | `link-oidc` |
| {handle} can no longer sign in through that identity. | `unlink-oidc` |
| {name} ({handle}) was given {role} on {repo}: {means} | `grant` |
| {name} ({handle}) no longer has the role {role} of its own on {repo}. | `revoke` |
| {repo} now has no administrator until you grant one. | `revoke --allow-no-admin` of the last one |
| These repositories will have no administrator until you grant one: {repos} | `disable`, before it goes ahead |
| These repositories will have no administrator until you grant one: {repos} and {more} more | the same, for a long list |
| A token for {handle} ({name}) was created. Its id is {id}. | `token create` |
| It is read-only: it can look at repositories and clone them, and nothing else. | the scope of a new token |
| It can read and propose work, but it cannot deliver a branch. | the scope of a new token |
| This token can do everything the account can, including delivering a branch. | the scope of a new token |
| It reaches every repository the account can reach. | the repositories of a new token |
| It reaches only these repositories: {repos}. | the repositories of a new token |
| It ends on {date}. | the end of a new token |
| This is the only time the token is shown. Keep it somewhere private: it cannot be shown again. | `token create` (not with `--out`) |
| The token was written to {file} (mode 0600). Give that path to planeter-client --token-file. | `token create --out` |
| Token {id} ({name}) of {handle} was revoked. It is refused from the next request. | `token revoke` |

### Every refusal, in its words

Each of these changes nothing. The exit code is the one of the table above (`1` unless it says `2`). The sentence is printed on standard error after "planeter operator:".

**The account commands**

| the sentence | when |
|---|---|
| The handle {handle} is not allowed. A handle is 1 to 39 characters: lowercase letters, digits, - _ and ., starting with a letter or digit, with no .. and no . at the end. | the handle does not follow the rule (exit 2) |
| A handle is written in lowercase letters, and {handle} has capitals. Write it in lowercase and run the command again. | capitals (exit 2) |
| {handle} is a word this server uses for its own pages. Choose another handle. | a reserved word |
| The handle {handle} is already in use (handles are compared without regard to case, and organizations and repository owners count). Choose another. | taken |
| A display name is required: --name TEXT, 1 to {max} characters. | no name (exit 2) |
| The display name is {length} characters long, and it can be at most {max}. Shorten it and run the command again. | too long (exit 2) |
| The display name has a character that cannot be shown, such as a line break. Write it as plain text and run the command again. | an unsafe name (exit 2) |
| The password is too short: use at least {min} characters. Nothing was changed. | the password rule |
| The password is too long: use at most {max} characters. Nothing was changed. | the password rule |
| The password is the same as the handle. Choose another. Nothing was changed. | the password rule |
| The password is one of the most common passwords. Choose another. Nothing was changed. | the password rule |
| The two entries are not the same. Nothing was changed; run the command again. | the two typed passwords differ |
| Standard input is not a terminal, so a password cannot be typed. Run the command in a terminal, or give the password on a file descriptor with --password-fd N. | no terminal (exit 2) |
| The file descriptor {fd} cannot be read as one line of password: {why}. Nothing was changed. | `--password-fd` |
| The terminal's echo could not be turned off ({why}), so no password was asked for. Use --password-fd N, or run the command where stty works. Nothing was changed. | no echo control (exit 2) |
| There is no account {handle}. Check the handle. | unknown handle |
| A reason is required: say why the account is disabled, in 1 to {max} characters. | `disable` without `--reason` (exit 2) |
| {handle} is already disabled. Nothing was changed. | `disable` twice |
| {handle} is not disabled. Nothing was changed. | `enable` on an active account |
| No sign-in provider is set up on this server (PLANETER_OIDC_ISSUER is not set), so there is no identity to link. Set it as the server does and run the command again. | `link-oidc` with no provider |
| {issuer} is not the sign-in provider this server is set up for. Use the value of PLANETER_OIDC_ISSUER. | `--issuer` is another provider |
| This identity is already linked to another account, and it is not moved. Unlink it there first. | `link-oidc` |
| This identity is not linked to {handle}. Nothing was changed. | `unlink-oidc` |
| This identity is already linked to {handle}. Nothing was changed. | `link-oidc` twice |
| The role must be read, write, maintain or admin. | `grant` (exit 2) |
| {handle} is disabled, so it cannot be given a role. Enable it first. | `grant` to a disabled account |
| That would leave {repo} with no administrator. Make another account an administrator first, or add --allow-no-admin to leave it without one. | `revoke` the last administrator |
| {handle} has no role of its own on {repo}, so there is nothing to take away. | `revoke` |
| {handle} already has the role {role} on {repo}. Nothing was changed. | `grant` twice |
| The database stayed busy for more than {seconds} seconds (another process is writing). Nothing was changed. Run the command again. | exit 5 |

**The token commands**

| the sentence | when |
|---|---|
| {handle} is disabled, so it cannot be given a token. Enable it first. | `create` for a disabled account |
| A name is required: --name TEXT, 1 to {max} characters, so that the token can be told from the others. | no name (exit 2) |
| The name is {length} characters long, and it can be at most {max}. Shorten it and run the command again. | too long (exit 2) |
| The name has a character that cannot be shown, such as a line break. Write it as plain text and run the command again. | an unsafe name (exit 2) |
| A scope is required: --scope read-only, write or full. | no scope (exit 2) |
| The scope must be read-only, write or full. | unknown scope (exit 2) |
| --expires-days must be a whole number of days from 1 to {max}. | exit 2 |
| A token can be limited to at most {max} repositories. | `--repos` (exit 2) |
| --repos needs one or more OWNER/NAME, separated by commas. | `--repos` (exit 2) |
| {handle} already has {max} tokens that have not expired. Revoke one first with planeter operator token revoke. | the bound |
| {file} already exists, and a token file is never overwritten. Choose a file that does not exist. Nothing was stored. | `--out` |
| The token could not be written to {file}: {why}. Nothing was stored. | `--out` (exit 5) |
| {handle} has no token {id}. Run planeter operator token list {handle} to see its tokens. | `revoke` |

A command that names a repository that does not exist (`grant`, `revoke`, `token create --repos`) says "There is no repository {repo}. Check the owner and the name."

**What the other lines of the output say.** A token list with nothing in it says "{handle} has no tokens." and an account list "There are no accounts."; a list longer than the program prints ends with "and {more} more of {total}".

### Sign-in providers

A server can offer **OpenID Connect sign-in** through one provider. Three settings turn it on (all three are needed) and one is optional:

| setting | what it is |
|---|---|
| `PLANETER_OIDC_ISSUER` | the provider's address (https) |
| `PLANETER_OIDC_CLIENT_ID` | the client id planeter has at the provider |
| `PLANETER_OIDC_REDIRECT_URI` | the address the provider sends the person back to |
| `PLANETER_OIDC_CLIENT_SECRET` | optional: the client secret, if the provider's client has one (it is never printed, logged or given to any command of this page) |

Two more are about how it behaves:

| setting | what it does |
|---|---|
| `PLANETER_OIDC_ALLOW_PRIVATE_ISSUER` | `on` or `off` (the default): see "A provider on a private address", below |
| `PLANETER_SESSION_HOURS` | how long a new sign-in lasts: 1 to 720 hours, default 168 (seven days); see "How long a sign-in lasts", below |

**What a provider link is.** A person signs in through the provider, and planeter finds the account **linked to that provider's identity** `(issuer, subject)`. An identity that is linked to no account is refused ("This single sign-on identity is not linked to a planeter account. Ask the person who runs this server."): the operator links it with `account link-oidc`. planeter makes no account for an unknown identity, and links nothing by name or by email.

**A wrong setting refuses the start**, with one of these sentences (never clamped and never ignored), and a right one prints a line at start-up:

| the sentence | when |
|---|---|
| PLANETER_OIDC_ALLOW_PRIVATE_ISSUER must be on or off, and it is {value}. Set it to on, or remove it. | any other value |
| PLANETER_OIDC_ALLOW_PRIVATE_ISSUER=on needs a sign-in provider. Set PLANETER_OIDC_ISSUER, or remove this setting. | on, with no issuer |
| PLANETER_OIDC_ISSUER {issuer} has no host that can be allowed. Check the address. | on, with an issuer that is not an https address with a host |
| PLANETER_SESSION_HOURS must be a whole number of hours from {min} to {max}, and it is {value}. Set a number in that range, or remove it. | outside 1 to 720 |

| the start-up line | when |
|---|---|
| The sign-in provider's host {host} is allowed to be on a private address (PLANETER_OIDC_ALLOW_PRIVATE_ISSUER=on). Every other outbound address is checked as before. | the allowance is on; {host} is the issuer's host, with `:port` when the port is not 443 |
| Sign-ins last {count} hour. ‖ Sign-ins last {count} hours. | always |

#### What the provider told us, and what planeter does about it

This is the provider planeter prefers, sui-id. What follows is what its architect **told** us from its source (the ledger's rows ID-1 to ID-10, each with its status and its limits there). **It is not a measurement, and it is not a claim about a release: planeter's interoperability test against a real release has not run, and this page says nothing like "tested with" a provider. A provider other than sui-id works if it speaks OpenID Connect the same way; planeter claims nothing about one it has not tested.**

* **ID-1.** The provider says `email_verified` is always false. planeter stores an email only when it is verified, so it stores **none** from this provider, which is the safe result.
* **ID-2.** The ID token carries no `name` and no `preferred_username` (those live at the userinfo endpoint), and its only algorithm is EdDSA. So planeter **reads neither**, asks for **no userinfo**, and the person's name and handle are the ones the operator typed. Nothing the provider says about a name can reach a page.
* **ID-3.** `acr` and `amr` say which factors were used, not when; planeter does not use them to judge how recent a sign-in was.
* **ID-4.** The provider **silently ignores `prompt` and `max_age`**, so planeter **sends neither**, and never reads a successful answer as proof of a recent sign-in.
* **ID-5.** The provider has no logout notification. Disabling a person at the provider ends their sign-ins *there*; planeter is not told. **A planeter sign-in outlives the provider's, so the operator disables a person in both places** (below).
* **ID-6.** The `sub` is a random identifier that is never reused; planeter stores it, with the issuer, as the identity. Everything else the provider says about a person is untrusted text, and is not read (ID-2).
* **ID-7.** The provider keeps its previous signing key in its key set until an administrator deletes it, and publishes a new key when it becomes the signer. planeter fetches the key set again once when it meets an unknown key, and **sends that request with `Cache-Control: no-cache`** so that a shared cache cannot answer with the old copy.
* **ID-8.** Nothing at the provider is under a stability promise before its 1.0, and no production-readiness milestone has started. planeter does not call it production-ready.
* **ID-9.** An embeddable form of the provider is a contract with no code. planeter does not embed an identity provider.
* **ID-10.** The interoperability test must run against a **tagged release**, with two added checks (`prompt=login` is not read as a recent sign-in; a person disabled at the provider ends the planeter sign-in, or the test reports how long it continues). **Open.**

#### A provider on a private address

planeter refuses to connect to a loopback, private, link-local or carrier-grade-NAT address for any request it makes on its own, and no setting loosens that, **except one**. A provider on the same host or network would otherwise be unreachable, so:

`PLANETER_OIDC_ALLOW_PRIVATE_ISSUER=on` lets **the issuer's host and port, that one authority and nothing else on that machine**, be on a private address. **The same host on another port is another service and stays refused.**

* **The risk.** You set the issuer, and a private service at that host and port is then reachable by planeter's provider calls. So the setting belongs on an authority you trust to be the provider and nothing else. A server that is not the provider, but listens on the same host and port, would be called as if it were.
* **What it does not change.** The address must be **https** (a plain `http` provider is refused); **every other host is judged as before**; every endpoint the provider's discovery document names (authorization, token and key set) must be on **that host and that port**, or the sign-in is refused; the address that was checked is the address connected to (nothing is re-resolved in between); **redirects are not followed**.
* **No private certificate authority yet.** A private-address provider usually has a private certificate. planeter verifies it with the system's certificate store; a setting for one more trusted authority is not built, so put the authority in the system store.
* The start-up line (above) says that the allowance is on and for which host and port.

#### How long a sign-in lasts

A planeter sign-in is independent of the provider's after it is made. `PLANETER_SESSION_HOURS` sets how long a **new** sign-in lasts, at both ways of signing in (a password and the provider), in the server and in the cookie together: 1 to 720 hours, **default 168 (seven days)**. A sign-in that exists keeps the length it was made with. **With a provider, 24 or fewer is recommended**: it is the longest a person stays signed in to planeter after being removed at the provider, unless you disable them here.

**A person disabled at the provider keeps their planeter sign-in until it ends. So the operator disables them in both places**: at the provider, and here with `account disable` (which ends their sign-ins at once and refuses their tokens). planeter does not check, from time to time, that a person still exists at the provider (below).

## For a person with an account

* **Your token comes from the operator.** There is no page where you make your own yet. Ask the person who runs the server for one: say the scope you need and, if you want it limited, the repositories. They give you a file (or a line), once. **Keep it private**: anyone who has it can do what it allows, as you.
* **Use it with `planeter-client`:** give the path of the file to `--token-file FILE` (or put the token in `PLANETER_TOKEN`; never on the command line). The client refuses a token file that others can read: `chmod 600` it. See [`client.md`](client.md), "The token".
* **What each scope lets the token do.** `read-only`: look at repositories and clone them. `write`: also propose work. `full`: everything your account can do, including delivering a branch. **A `write` token cannot deliver**, whoever you are; a delivery needs a `full` token of an account with the `maintain` role (or `admin`) on the repository ([`deliveries.md`](deliveries.md)). A token never reaches further than your own roles do.
* **It ends.** Every token has an end (up to 365 days, 90 by default). When it ends, or when the operator revokes it, the client's next request is refused and you ask for another. If your account is disabled, your tokens are refused until it is enabled again, and a sign-in through the provider shows: "This account has been disabled. Ask the person who runs this server."
* **Your log.** Everything done to your account is recorded with who did it, in the sentences above, and never with a secret. You cannot read it on a page yet; the operator can show it to you (`account show`), and will say who made a token and when.
* **Who to ask.** The person who runs the server, for a token, a password, a role, a link to your identity at the provider, or anything about a disabled account. If you sign in through the provider and cannot sign in, the provider's own help comes first.

## What is not here yet

Accounts and tokens made by the operator exist. **Not built:** pages where a person makes their own tokens, changes their password, sees and ends their sign-ins and reads their security log (step 2); provisioning (an account made on first sign-in) and a collaborators page for a repository's administrators (step 3); a **second factor** (that is the provider's job, and planeter will not build one); **email recovery** (never in planeter); a **periodic check that a person still exists at the provider** (an open question of the owner's; until it is decided, a short sign-in length and disabling in both places are the way); the lifecycle of **organizations** (RFC 015, later); and **deleting an account** (never: disable). A private certificate authority for the provider is not built either (above).
