//! The command line: hand-written (no parser crate), long options only, `--option value` or `--option=value`. Every refusal is
//! a plain sentence and exit code 2.

use std::path::PathBuf;

use crate::catalog::{Key, Lang, t};
use crate::exit::{Failure, USAGE};

/// The options every command takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Common {
    pub token_file: Option<PathBuf>,
    pub cacert: Option<PathBuf>,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposeArgs {
    pub url: String,
    pub git_ref: String,
    pub proposal: Option<String>,
    pub title: Option<String>,
    pub description_file: Option<PathBuf>,
    pub dry_run: bool,
    pub retry: bool,
    pub common: Common,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneArgs {
    pub url: String,
    pub dir: Option<PathBuf>,
    pub trust_keys: Vec<String>,
    pub no_adopt: bool,
    pub common: Common,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchArgs {
    pub dir: Option<PathBuf>,
    pub common: Common,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Main,
    Propose,
    Clone,
    Fetch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help(Topic),
    Version,
    Propose(ProposeArgs),
    Clone(CloneArgs),
    Fetch(FetchArgs),
}

/// What was parsed: the language is known before the command is, so that a usage error can be worded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub lang: Option<Lang>,
    pub command: Command,
}

struct Cursor {
    items: Vec<String>,
    at: usize,
}

impl Cursor {
    fn next(&mut self) -> Option<String> {
        let v = self.items.get(self.at).cloned();
        self.at += 1;
        v
    }
}

fn usage(lang: Lang, key: Key) -> crate::catalog::Msg {
    t(lang, key)
}

/// Split `--name=value`.
fn split_eq(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((n, v)) if n.starts_with("--") => (n, Some(v)),
        _ => (arg, None),
    }
}

/// Parse the arguments after the program name. `lang_hint` is the language to use for usage errors until `--lang` is seen.
pub fn parse(args: &[String], lang_hint: Lang) -> Result<Parsed, Failure> {
    // First pass: `--lang`, wherever it stands, so that every later message is in it.
    let mut lang: Option<Lang> = None;
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        let (name, inline) = split_eq(a);
        if name == "--lang" {
            let value = inline.map(str::to_owned).or_else(|| it.next().cloned());
            match value.as_deref().and_then(Lang::from_flag) {
                Some(l) => lang = Some(l),
                None => {
                    let shown = value.unwrap_or_default();
                    return Err(Failure::new(
                        USAGE,
                        usage(lang_hint, Key::UsageBadLang)
                            .with("value", shown)
                            .text(),
                    ));
                }
            }
        }
    }
    let l = lang.unwrap_or(lang_hint);
    let mut cur = Cursor {
        items: args.to_vec(),
        at: 0,
    };
    let mut command_name: Option<String> = None;
    // The command is the first word that is not an option (or an option's value); find it without consuming options of it.
    let mut rest: Vec<String> = Vec::new();
    let mut wants_help = false;
    let mut wants_version = false;
    while let Some(a) = cur.next() {
        match a.as_str() {
            "--help" | "-h" => wants_help = true,
            "--version" | "-V" => wants_version = true,
            // `--lang` was read above; its value is not the command.
            "--lang" => {
                cur.next();
            }
            _ if a.starts_with("--lang=") => {}
            _ if command_name.is_none() && !a.starts_with('-') => command_name = Some(a),
            _ => rest.push(a),
        }
    }
    // Options before the command that take a value ride along in `rest` and are parsed by the command; global ones too.
    let topic_of = |name: &str| match name {
        "propose" => Some(Topic::Propose),
        "clone" => Some(Topic::Clone),
        "fetch" => Some(Topic::Fetch),
        _ => None,
    };
    if wants_version && command_name.is_none() {
        return Ok(Parsed {
            lang,
            command: Command::Version,
        });
    }
    let Some(name) = command_name else {
        if wants_help || rest.is_empty() {
            return Ok(Parsed {
                lang,
                command: Command::Help(Topic::Main),
            });
        }
        let first = rest[0].clone();
        return Err(Failure::new(
            USAGE,
            usage(l, Key::UsageUnknownOption)
                .with("name", first)
                .with("command", "")
                .text(),
        ));
    };
    let Some(topic) = topic_of(&name) else {
        return Err(Failure::new(
            USAGE,
            usage(l, Key::UsageUnknownCommand).with("name", name).text(),
        ));
    };
    if wants_help {
        return Ok(Parsed {
            lang,
            command: Command::Help(topic),
        });
    }
    let command = match topic {
        Topic::Propose => Command::Propose(parse_propose(&rest, l)?),
        Topic::Clone => Command::Clone(parse_clone(&rest, l)?),
        Topic::Fetch => Command::Fetch(parse_fetch(&rest, l)?),
        Topic::Main => unreachable!("main is not a command"),
    };
    Ok(Parsed { lang, command })
}

/// Option reader shared by the commands.
struct Opts<'a> {
    args: &'a [String],
    at: usize,
    lang: Lang,
    command: &'static str,
}

enum Item {
    Flag(String),
    Value(String, String),
    Positional(String),
}

impl<'a> Opts<'a> {
    fn new(args: &'a [String], lang: Lang, command: &'static str) -> Self {
        Opts {
            args,
            at: 0,
            lang,
            command,
        }
    }

    /// The next item; `takes_value` says which options take one.
    fn next(&mut self, takes_value: &[&str]) -> Option<Result<Item, Failure>> {
        let a = self.args.get(self.at)?.clone();
        self.at += 1;
        if !a.starts_with("--") {
            return Some(Ok(Item::Positional(a)));
        }
        let (name, inline) = split_eq(&a);
        if takes_value.contains(&name) {
            let value = match inline {
                Some(v) => Some(v.to_owned()),
                None => {
                    let v = self.args.get(self.at).cloned();
                    if v.is_some() {
                        self.at += 1;
                    }
                    v
                }
            };
            return Some(match value {
                Some(v) => Ok(Item::Value(name.to_owned(), v)),
                None => Err(Failure::new(
                    USAGE,
                    usage(self.lang, Key::UsageMissingValue)
                        .with("name", name)
                        .with("command", self.command)
                        .text(),
                )),
            });
        }
        Some(Ok(Item::Flag(name.to_owned())))
    }

    fn unknown(&self, name: &str) -> Failure {
        Failure::new(
            USAGE,
            usage(self.lang, Key::UsageUnknownOption)
                .with("name", name)
                .with("command", self.command)
                .text(),
        )
    }

    fn too_many(&self, extra: &str) -> Failure {
        Failure::new(
            USAGE,
            usage(self.lang, Key::UsageTooMany)
                .with("extra", extra)
                .with("command", self.command)
                .text(),
        )
    }

    fn missing(&self, what: &str) -> Failure {
        Failure::new(
            USAGE,
            usage(self.lang, Key::UsageMissingArgument)
                .with("what", what)
                .with("command", self.command)
                .text(),
        )
    }
}

const COMMON_VALUES: [&str; 3] = ["--token-file", "--cacert", "--lang"];

fn common_item(common: &mut Common, name: &str, value: Option<&str>) -> bool {
    match (name, value) {
        ("--token-file", Some(v)) => common.token_file = Some(PathBuf::from(v)),
        ("--cacert", Some(v)) => common.cacert = Some(PathBuf::from(v)),
        ("--lang", Some(_)) => {}
        ("--verbose", None) => common.verbose = true,
        _ => return false,
    }
    true
}

fn parse_propose(args: &[String], lang: Lang) -> Result<ProposeArgs, Failure> {
    let values: Vec<&str> = COMMON_VALUES
        .iter()
        .copied()
        .chain(["--ref", "--proposal", "--title", "--description-file"])
        .collect();
    let mut o = Opts::new(args, lang, "propose");
    let mut out = ProposeArgs {
        url: String::new(),
        git_ref: "heads/main".to_owned(),
        proposal: None,
        title: None,
        description_file: None,
        dry_run: false,
        retry: false,
        common: Common::default(),
    };
    let mut url: Option<String> = None;
    while let Some(item) = o.next(&values) {
        match item? {
            Item::Positional(p) => {
                if url.is_none() {
                    url = Some(p);
                } else {
                    return Err(o.too_many(&p));
                }
            }
            Item::Value(n, v) => match n.as_str() {
                "--ref" => out.git_ref = v,
                "--proposal" => out.proposal = Some(v),
                "--title" => out.title = Some(v),
                "--description-file" => out.description_file = Some(PathBuf::from(v)),
                _ => {
                    if !common_item(&mut out.common, &n, Some(&v)) {
                        return Err(o.unknown(&n));
                    }
                }
            },
            Item::Flag(n) => match n.as_str() {
                "--dry-run" => out.dry_run = true,
                "--retry" => out.retry = true,
                _ => {
                    if !common_item(&mut out.common, &n, None) {
                        return Err(o.unknown(&n));
                    }
                }
            },
        }
    }
    out.url = url.ok_or_else(|| o.missing("the address of the repository"))?;
    Ok(out)
}

fn parse_clone(args: &[String], lang: Lang) -> Result<CloneArgs, Failure> {
    let values: Vec<&str> = COMMON_VALUES
        .iter()
        .copied()
        .chain(["--trust-key"])
        .collect();
    let mut o = Opts::new(args, lang, "clone");
    let mut out = CloneArgs {
        url: String::new(),
        dir: None,
        trust_keys: Vec::new(),
        no_adopt: false,
        common: Common::default(),
    };
    let mut url: Option<String> = None;
    while let Some(item) = o.next(&values) {
        match item? {
            Item::Positional(p) => {
                if url.is_none() {
                    url = Some(p);
                } else if out.dir.is_none() {
                    out.dir = Some(PathBuf::from(p));
                } else {
                    return Err(o.too_many(&p));
                }
            }
            Item::Value(n, v) => match n.as_str() {
                "--trust-key" => out.trust_keys.push(v),
                _ => {
                    if !common_item(&mut out.common, &n, Some(&v)) {
                        return Err(o.unknown(&n));
                    }
                }
            },
            Item::Flag(n) => match n.as_str() {
                "--no-adopt" => out.no_adopt = true,
                _ => {
                    if !common_item(&mut out.common, &n, None) {
                        return Err(o.unknown(&n));
                    }
                }
            },
        }
    }
    out.url = url.ok_or_else(|| o.missing("the address of the repository"))?;
    Ok(out)
}

fn parse_fetch(args: &[String], lang: Lang) -> Result<FetchArgs, Failure> {
    let mut o = Opts::new(args, lang, "fetch");
    let mut out = FetchArgs {
        dir: None,
        common: Common::default(),
    };
    while let Some(item) = o.next(&COMMON_VALUES) {
        match item? {
            Item::Positional(p) => {
                if out.dir.is_none() {
                    out.dir = Some(PathBuf::from(p));
                } else {
                    return Err(o.too_many(&p));
                }
            }
            Item::Value(n, v) => {
                if !common_item(&mut out.common, &n, Some(&v)) {
                    return Err(o.unknown(&n));
                }
            }
            Item::Flag(n) => {
                if !common_item(&mut out.common, &n, None) {
                    return Err(o.unknown(&n));
                }
            }
        }
    }
    Ok(out)
}
