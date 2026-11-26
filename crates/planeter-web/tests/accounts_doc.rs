//! RFC 014 step 1, handoff 5: `docs/accounts.md` says only what the catalog, the ledger and the code hold. The sentences it quotes are the
//! catalog's, word for word (every form of every one, so that a message added to a family the page quotes fails here until the page says it);
//! every command and every flag the help lists is on the page; every setting the server reads for accounts and providers is on the page, and
//! no setting is on the page that nothing reads; the numbers are the code's; the ledger rows it cites exist, and the page claims no test of a
//! provider that has not run; it never says the words the language rules retire; and the links it makes lead somewhere. It is the twin of
//! `keys_doc.rs`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use planeter_web::catalog::shipped;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn doc() -> String {
    read("docs/accounts.md")
}

/// Every English message: its key and each of its forms.
fn english() -> Vec<(&'static str, Vec<&'static str>)> {
    shipped()
        .iter()
        .find(|l| l.tag == "en")
        .unwrap()
        .entries
        .iter()
        .map(|e| (e.key, e.forms().map(|(_, t)| t).collect()))
        .collect()
}

fn forms(key: &str) -> Vec<&'static str> {
    english()
        .into_iter()
        .find(|(k, _)| *k == key)
        .unwrap_or_else(|| panic!("no message {key}"))
        .1
}

/// The text with its code blocks and inline code taken out: what a reader reads as prose.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
            } else if !in_code {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

/// `PLANETER_…` names in `text`.
fn names_in(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find("PLANETER_") {
        let start = i + off;
        let mut end = start + "PLANETER_".len();
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        found.insert(text[start..end].trim_end_matches('_').to_owned());
        i = end;
    }
    found
}

/// The messages of these families are on the page, **every form of each**, except the ones named: a message added to a family must be said.
/// (`account.event.role.*` are the four words of `--role`; `account.event.then` is the joining of two sentences.)
const QUOTED_FAMILIES: &[&str] = &[
    "operator.account.error.",
    "operator.account.done.",
    "operator.account.warn.",
    "operator.account.role.",
    "operator.account.note.",
    "operator.token.error.",
    "operator.token.done.",
    "operator.token.scope.",
    "operator.token.repos.",
    "provider.error.",
    "provider.line.",
    "account.event.",
    "account.by.",
];

/// Not quoted, with the reason.
const NOT_QUOTED: &[(&str, &str)] = &[
    (
        "account.event.then",
        "two sentences joined; the page shows the joined result",
    ),
    (
        "account.event.role.read",
        "one word: the page lists the four words of --role",
    ),
    (
        "account.event.role.write",
        "one word: the page lists the four words of --role",
    ),
    (
        "account.event.role.maintain",
        "one word: the page lists the four words of --role",
    ),
    (
        "account.event.role.admin",
        "one word: the page lists the four words of --role",
    ),
    (
        "account.event.token_repositories",
        "quoted in running text, as a plural pair",
    ),
    (
        "account.event.token_repositories_all",
        "quoted in running text",
    ),
    (
        "account.by.self",
        "one word: quoted as \"you\" in running text",
    ),
    (
        "account.by.operator",
        "quoted as \"this server's operator\" in running text",
    ),
    (
        "account.by.provider",
        "quoted as \"the sign-in provider\" in running text",
    ),
];

#[test]
fn the_page_quotes_the_catalogs_sentences_word_for_word() {
    let doc = doc();
    let skipped: BTreeSet<&str> = NOT_QUOTED.iter().map(|(k, _)| *k).collect();
    let mut checked = 0;
    for (key, texts) in english() {
        if !QUOTED_FAMILIES.iter().any(|p| key.starts_with(p)) {
            continue;
        }
        if skipped.contains(key) {
            continue;
        }
        for text in texts {
            // The refusal of an unknown repository and the usage line are quoted once each, below. A sentence that names the page's own
            // placeholder is quoted as the catalog writes it, braces included.
            assert!(
                doc.contains(text),
                "{key}: {text:?} is not in docs/accounts.md"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 80,
        "the families are quoted: {checked} sentences checked"
    );

    // The exempt ones that are quoted in running text, quoted as the catalog writes them.
    for key in [
        "account.event.token_repositories",
        "account.event.token_repositories_all",
    ] {
        for text in forms(key) {
            assert!(doc.contains(text), "{key}: {text:?}");
        }
    }
    for word in ["you", "this server's operator", "the sign-in provider"] {
        assert!(doc.contains(&format!("\"{word}\"")), "{word}");
    }
    for (key, word) in [
        ("account.by.self", "you"),
        ("account.by.operator", "this server's operator"),
        ("account.by.provider", "the sign-in provider"),
    ] {
        assert_eq!(forms(key), vec![word], "{key}");
    }
    for key in [
        "account.event.role.read",
        "account.event.role.write",
        "account.event.role.maintain",
        "account.event.role.admin",
    ] {
        let word = forms(key)[0];
        assert!(doc.contains(&format!("\"{word}\"")), "{key}: {word:?}");
    }

    // The sentences of the other families the page quotes: the usage refusals, the unknown repository, the pages a person sees, the lists.
    for key in [
        "operator.account.error.usage",
        "operator.token.error.usage",
        "operator.error.unknown_repo",
        "page.sso_unlinked.body",
        "page.sso_disabled.body",
        "login.passwords_off",
        "operator.account.list.header",
        "operator.account.list.none",
        "operator.account.list.more",
        "operator.account.check.none",
        "operator.token.list.header",
        "operator.token.list.none",
    ] {
        for text in forms(key) {
            assert!(
                doc.contains(text),
                "{key}: {text:?} is not in docs/accounts.md"
            );
        }
    }
    // `disable` says that nothing was deleted and promises nothing about who can read what (a private repository is not readable by others).
    let disabled = forms("operator.account.done.disabled")[0];
    assert!(
        disabled.contains("Nothing of the account was deleted") && !disabled.contains("others"),
        "{disabled}"
    );
    assert!(!doc.contains("others can still read"));
    // The three-line outputs of the examples are the catalog's own sentences, with the example's values.
    for key in [
        "operator.account.done.sessions_ended",
        "operator.account.done.disabled",
        "operator.account.done.enabled",
        "operator.token.done.once",
    ] {
        assert!(doc.contains(forms(key)[0]), "{key}");
    }
}

#[test]
fn the_exemption_list_names_only_messages_that_exist() {
    let have: BTreeSet<&str> = english().iter().map(|(k, _)| *k).collect();
    for (key, why) in NOT_QUOTED {
        assert!(have.contains(key), "{key} ({why}) is not a message");
    }
}

#[test]
fn every_command_and_flag_the_help_lists_is_on_the_page() {
    let doc = doc();
    for (key, family) in [
        ("operator.account.help.usage", "account"),
        ("operator.token.help.usage", "token"),
    ] {
        let help = forms(key)[0];
        let usage: Vec<&str> = help
            .lines()
            .filter(|l| l.contains("planeter operator"))
            .collect();
        assert!(usage.len() >= 3, "{key}: the usage lines are read");
        let mut commands = BTreeSet::new();
        for line in usage {
            let rest = line.split("planeter operator ").nth(1).unwrap();
            let mut words = rest.split_whitespace();
            assert_eq!(words.next(), Some(family));
            commands.insert(words.next().unwrap().to_owned());
        }
        assert!(commands.len() >= 3, "{key}: {commands:?}");
        for command in &commands {
            assert!(
                doc.contains(&format!("`{family} {command}"))
                    || doc.contains(&format!("{family} {command}")),
                "{key}: the command `{family} {command}` is not on the page"
            );
        }
        // every flag, wherever the help names it
        let mut flags = BTreeSet::new();
        for word in help.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
            if word.starts_with("--") && word.len() > 2 {
                flags.insert(word.to_owned());
            }
        }
        for flag in &flags {
            assert!(
                doc.contains(flag.as_str()),
                "{key}: the flag {flag} is not on the page"
            );
        }
        assert!(flags.contains("--name"), "{key}: the flags are read");
    }
    // the five exit codes of the family, as the help says them
    let exits = forms("operator.account.help.exit_codes")[0];
    assert!(exits.contains("3 is not used") && exits.contains("4 is not used"));
    // the token family's help says its own exit codes: no terminal, no password
    let exits = forms("operator.token.help.exit_codes")[0];
    assert!(exits.contains("3 is not used") && exits.contains("4 is not used"));
    assert!(
        !exits.contains("password") && !exits.contains("terminal"),
        "{exits}"
    );
    assert!(
        forms("operator.account.help.exit_codes")[0].contains("no terminal for a password"),
        "the account family's help still says it"
    );
    assert!(doc.contains("**`3` and `4` are not used**"));
    for code in ["`0`", "`1`", "`2`", "`5`"] {
        assert!(doc.contains(code), "{code}");
    }
}

#[test]
fn the_page_names_every_setting_the_server_reads_for_accounts_and_providers_and_no_other() {
    let doc = doc();
    // What the server and the account commands read, from the source (outside comments): the sign-in provider's settings, the length of a
    // sign-in, and the database the commands open.
    let mut read = BTreeSet::new();
    for file in [
        "crates/planeter/src/provider_env.rs",
        "crates/planeter/src/operator_account.rs",
        "crates/planeter/src/args.rs",
        "crates/planeter/src/main.rs",
        "crates/planeter/src/config.rs",
        "crates/planeter-auth/src/session.rs",
        "crates/planeter-web/src/authn.rs",
    ] {
        let code: String = read_text(file)
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for name in names_in(&code) {
            if name.starts_with("PLANETER_OIDC_")
                || name.starts_with("PLANETER_SESSION_")
                || name == "PLANETER_LOCAL_PASSWORDS"
            {
                read.insert(name);
            }
        }
    }
    for expected in [
        "PLANETER_OIDC_ISSUER",
        "PLANETER_OIDC_CLIENT_ID",
        "PLANETER_OIDC_REDIRECT_URI",
        "PLANETER_OIDC_CLIENT_SECRET",
        "PLANETER_OIDC_ALLOW_PRIVATE_ISSUER",
        "PLANETER_SESSION_HOURS",
        "PLANETER_LOCAL_PASSWORDS",
    ] {
        assert!(
            read.contains(expected),
            "the scan found no {expected}: {read:?}"
        );
    }
    let on_page = names_in(&doc);
    let missing: Vec<_> = read.difference(&on_page).collect();
    assert!(
        missing.is_empty(),
        "read by the server but not on the page: {missing:?}"
    );
    // and every name on the page is one something reads (the server, the commands or the client)
    let mut sources = String::new();
    for dir in [
        "crates/planeter/src",
        "crates/planeter-client/src",
        "crates/planeter-auth/src",
        "crates/planeter-web/src",
    ] {
        collect(&root().join(dir), &mut sources);
    }
    let known = names_in(&sources);
    let stale: Vec<_> = on_page.difference(&known).collect();
    assert!(
        stale.is_empty(),
        "on the page but read by nothing: {stale:?}"
    );
    // the settings that are for the commands
    assert!(on_page.contains("PLANETER_DB"));
}

fn read_text(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn collect(dir: &std::path::Path, out: &mut String) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push_str(&std::fs::read_to_string(path).unwrap());
            out.push('\n');
        }
    }
}

#[test]
fn the_numbers_on_the_page_are_the_codes() {
    let doc = doc();
    assert!(doc.contains(&format!(
        "At least **{}** and at most **{}** characters",
        planeter_auth::password::PASSWORD_MIN,
        planeter_auth::password::PASSWORD_MAX
    )));
    assert!(doc.contains(&format!(
        "{} to {} hours, default {}",
        planeter_auth::SESSION_HOURS_MIN,
        planeter_auth::SESSION_HOURS_MAX,
        planeter_auth::SESSION_HOURS_DEFAULT
    )));
    assert_eq!(planeter_auth::SESSION_HOURS_DEFAULT, 7 * 24);
    assert!(doc.contains("(seven days)"));
    assert!(doc.contains(&format!(
        "**At most {} tokens that have not expired**",
        planeter_auth::TOKENS_MAX
    )));
    assert!(doc.contains(&format!(
        "**Name.** 1 to {} characters",
        planeter_auth::TOKEN_NAME_MAX
    )));
    assert!(doc.contains(&format!(
        "Text, 1 to {} characters",
        planeter_auth::DISPLAY_NAME_MAX
    )));
    assert!(doc.contains(&format!(
        "1 to {} characters; no `..`",
        planeter_core::creation::NEW_HANDLE_MAX
    )));
    // the figures only the help states: the token's days and repositories, the reason's length
    let tokens = forms("operator.token.help.usage")[0];
    assert!(tokens.contains("1 to 365 (default 90)") && tokens.contains("at most 20"));
    assert!(doc.contains("1 to 365 days (`--expires-days`, default 90)"));
    assert!(doc.contains("at most 20 repositories"));
    let account = forms("operator.account.help.usage")[0];
    assert!(account.contains("1 to 200 characters"));
    assert!(doc.contains("(1 to 200 characters,"));
    // the bounds the start-up refusal prints are the code's
    let refusal = forms("provider.error.session_hours")[0];
    assert!(refusal.contains("{min}") && refusal.contains("{max}"));
}

#[test]
fn the_ledger_rows_it_cites_exist_and_nothing_stronger_than_they_say_is_claimed() {
    let doc = doc();
    let ledger = read("docs/src/planeter-idp-ledger.md");
    for n in 1..=10 {
        let id = format!("ID-{n}");
        assert!(
            doc.contains(&format!("**{id}.**")),
            "{id} is cited with its own sentence"
        );
        assert!(
            ledger.contains(&format!("| {id} |")),
            "{id} is in the ledger"
        );
    }
    // ID-10 is open in the ledger, and the page says so
    let id10 = ledger.lines().find(|l| l.starts_with("| ID-10 |")).unwrap();
    assert!(id10.contains("OPEN"));
    assert!(doc.contains("**Open.**"));
    // Every other row is "TOLD", not measured: the page says "told", and says it is not a measurement
    for n in 1..=9 {
        let row = ledger
            .lines()
            .find(|l| l.starts_with(&format!("| ID-{n} |")))
            .unwrap();
        assert!(row.contains("TOLD"), "ID-{n} is TOLD");
    }
    assert!(doc.contains("**told**") && doc.contains("It is not a measurement"));
    // The only sentence the documents may use about a tested provider is "tested with sui-id {version} on {date}", and none is true yet:
    // the page names no release and makes no such claim.
    let lower = doc.to_lowercase();
    assert!(
        !lower.contains("tested with sui-id"),
        "a 'tested with' claim"
    );
    assert!(
        !lower.contains("production-ready") || lower.contains("does not call it production-ready")
    );
    for (i, _) in lower.match_indices("sui-id") {
        let after: Vec<char> = lower[i + "sui-id".len()..].chars().take(3).collect();
        let digit = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_digit());
        let versioned = {
            let mut rest = after.iter().skip_while(|c| **c == ' ');
            match rest.next() {
                Some('v') => digit(rest.next()),
                other => digit(other),
            }
        };
        assert!(!versioned, "sui-id followed by a version: {after:?}");
    }
    assert!(ledger.contains("none is true yet"));
}

#[test]
fn it_says_nothing_about_the_development_machine() {
    let lower = doc().to_lowercase();
    for word in [
        "run-capped",
        "gates.sh",
        "development machine",
        "development-machine",
        "cpuquota",
    ] {
        assert!(!lower.contains(word), "{word}");
    }
}

#[test]
fn it_never_uses_the_words_the_language_rules_retire() {
    let doc = doc();
    let text = prose(&doc);
    // The retired words of the source language (tests/i18n/en.rules): in a user-facing sentence a person says "sign-in", never "session".
    let lower = text.to_lowercase();
    for word in ["session", "sessions"] {
        assert!(
            !contains_word(&lower, word),
            "{word} is retired: say sign-in ({:?})",
            context(&lower, word)
        );
    }
    for word in [
        "SSO",
        "refs",
        "unsealed",
        "forge-approved",
        "prikk-verified",
    ] {
        assert!(!contains_word(&text, word), "{word} is retired");
    }
    // The setting's own name keeps its word: it is an identifier, not a sentence.
    assert!(doc.contains("PLANETER_SESSION_HOURS"));
    // "retired", "revoked", "expired" and "rotated" are never said about a key (a key is never removed or marked: docs/keys.md).
    for line in doc.lines() {
        let l = line.to_lowercase();
        let about_a_key = contains_word(&l, "key") || contains_word(&l, "keys");
        if about_a_key {
            for word in ["retired", "revoked", "rotated", "expired", "removed"] {
                assert!(
                    !contains_word(&l, word),
                    "{word} on a line about keys: {line}"
                );
            }
        }
    }
}

/// The inline code spans, outside fenced blocks.
fn code_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            spans.extend(line.split('`').skip(1).step_by(2).map(str::to_owned));
        }
    }
    spans
}

fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

fn context(text: &str, word: &str) -> String {
    text.lines()
        .find(|l| contains_word(l, word))
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

#[test]
fn the_links_it_makes_lead_somewhere_and_the_other_pages_lead_to_it() {
    let doc = doc();
    // markdown links: (target)
    let mut links = 0;
    for (i, _) in doc.match_indices("](") {
        let rest = &doc[i + 2..];
        let target = &rest[..rest.find(')').unwrap()];
        if target.starts_with("http") {
            continue;
        }
        let path = target.split('#').next().unwrap();
        assert!(
            root().join("docs").join(path).exists(),
            "link {target:?} does not lead to a file under docs/"
        );
        links += 1;
    }
    assert!(links >= 3, "the page links to keys, client and deliveries");
    // paths named in code spans: they exist
    let mut named = 0;
    for part in code_spans(&doc) {
        if (part.starts_with("docs/") || part.starts_with("rfcs/") || part.starts_with("crates/"))
            && (part.ends_with(".md") || part.ends_with(".rs"))
        {
            assert!(root().join(&part).exists(), "{part} does not exist");
            named += 1;
        }
    }
    assert!(named >= 3, "{named}");
    // the other pages point to it
    assert!(read("README.md").contains("docs/accounts.md"));
    assert!(read("docs/client.md").contains("accounts.md"));
    assert!(read("docs/deliveries.md").contains("accounts.md"));
}

#[test]
fn the_readme_no_longer_lists_what_exists_as_not_yet_and_still_lists_what_does_not() {
    let readme = read("README.md");
    let not_yet = readme
        .lines()
        .skip_while(|l| !l.starts_with("- **Not yet:**"))
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !not_yet.contains("accounts and sign-in management (RFC 014, planned)"),
        "{not_yet}"
    );
    assert!(not_yet.contains("self-service pages"), "{not_yet}");
}
