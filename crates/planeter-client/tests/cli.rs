//! The command line: every refusal is one sentence and exit code 2; the options of each command; `--lang` anywhere.

use planeter_client::catalog::Lang;
use planeter_client::cli::{Command, Topic, parse};
use planeter_client::exit::USAGE;

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn propose_takes_its_options_and_defaults_to_heads_main() {
    let p = parse(
        &args(&[
            "propose",
            "https://f.example/a/b",
            "--title",
            "T",
            "--dry-run",
        ]),
        Lang::En,
    )
    .unwrap();
    let Command::Propose(a) = p.command else {
        panic!()
    };
    assert_eq!(
        (
            a.url.as_str(),
            a.git_ref.as_str(),
            a.title.as_deref(),
            a.dry_run,
            a.retry
        ),
        (
            "https://f.example/a/b",
            "heads/main",
            Some("T"),
            true,
            false
        )
    );
    let p = parse(
        &args(&[
            "propose",
            "u",
            "--ref=heads/x",
            "--proposal",
            "c-1",
            "--description-file",
            "d.md",
            "--retry",
            "--token-file",
            "t",
            "--cacert",
            "c.pem",
            "--verbose",
        ]),
        Lang::En,
    )
    .unwrap();
    let Command::Propose(a) = p.command else {
        panic!()
    };
    assert_eq!(a.git_ref, "heads/x");
    assert_eq!(a.proposal.as_deref(), Some("c-1"));
    assert!(a.retry && a.common.verbose);
    assert_eq!(a.common.token_file.unwrap().to_str(), Some("t"));
    assert_eq!(a.common.cacert.unwrap().to_str(), Some("c.pem"));
}

#[test]
fn clone_takes_a_directory_and_any_number_of_keys() {
    let p = parse(
        &args(&[
            "clone",
            "https://f.example/a/b",
            "here",
            "--trust-key",
            "aa",
            "--trust-key=bb",
            "--no-adopt",
        ]),
        Lang::En,
    )
    .unwrap();
    let Command::Clone(a) = p.command else {
        panic!()
    };
    assert_eq!(a.dir.unwrap().to_str(), Some("here"));
    assert_eq!(a.trust_keys, ["aa", "bb"]);
    assert!(a.no_adopt);
}

#[test]
fn fetch_takes_an_optional_directory() {
    assert!(
        matches!(parse(&args(&["fetch"]), Lang::En).unwrap().command, Command::Fetch(a) if a.dir.is_none())
    );
    assert!(
        matches!(parse(&args(&["fetch", "d"]), Lang::En).unwrap().command, Command::Fetch(a) if a.dir.is_some())
    );
}

#[test]
fn help_and_version_need_no_network_and_stand_anywhere() {
    assert!(matches!(
        parse(&args(&[]), Lang::En).unwrap().command,
        Command::Help(Topic::Main)
    ));
    assert!(matches!(
        parse(&args(&["--help"]), Lang::En).unwrap().command,
        Command::Help(Topic::Main)
    ));
    assert!(matches!(
        parse(&args(&["propose", "--help"]), Lang::En)
            .unwrap()
            .command,
        Command::Help(Topic::Propose)
    ));
    assert!(matches!(
        parse(&args(&["clone", "x", "-h"]), Lang::En)
            .unwrap()
            .command,
        Command::Help(Topic::Clone)
    ));
    assert!(matches!(
        parse(&args(&["--version"]), Lang::En).unwrap().command,
        Command::Version
    ));
}

#[test]
fn the_lang_option_stands_anywhere_and_words_the_usage_errors() {
    let p = parse(&args(&["--lang", "ja", "fetch"]), Lang::En).unwrap();
    assert_eq!(p.lang, Some(Lang::Ja));
    let p = parse(&args(&["fetch", "--lang=ja"]), Lang::En).unwrap();
    assert_eq!(p.lang, Some(Lang::Ja));
    let e = parse(&args(&["--lang", "ja", "nonsense"]), Lang::En).unwrap_err();
    assert_eq!(e.code, USAGE);
    assert!(
        e.message.contains("nonsense") && e.message.contains("コマンド"),
        "{}",
        e.message
    );
    let e = parse(&args(&["--lang", "fr", "fetch"]), Lang::En).unwrap_err();
    assert_eq!(e.code, USAGE);
}

#[test]
fn every_wrong_command_line_is_one_sentence_and_code_2() {
    for bad in [
        vec!["bogus"],
        vec!["propose"],
        vec!["propose", "u", "extra"],
        vec!["propose", "u", "--title"],
        vec!["propose", "u", "--nope"],
        vec!["clone"],
        vec!["clone", "u", "d", "extra"],
        vec!["fetch", "a", "b"],
        vec!["fetch", "--token", "SECRET"],
        vec!["--verbose"],
    ] {
        let e = parse(&args(&bad), Lang::En).unwrap_err();
        assert_eq!(e.code, USAGE, "{bad:?}");
        assert!(
            !e.message.is_empty() && !e.message.contains("SECRET") || bad.contains(&"--token"),
            "{bad:?}"
        );
    }
    // A token is never taken from the command line: `--token SECRET` is an unknown option, and the value is not echoed as a value.
    let e = parse(&args(&["fetch", "--token", "SECRET"]), Lang::En).unwrap_err();
    assert!(e.message.contains("--token"), "{}", e.message);
}
