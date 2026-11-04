//! The message catalog (RFC 004 part 9, T4): every sentence the program says, keyed by a stable identifier, one compiled-in table
//! per language, whole sentences with **named placeholders** (`{name}`). The same shape as the server's (`planeter-web`'s
//! `catalog`), smaller: no plural forms (a sentence with a count is written so that it reads right for one and for many) and no
//! markup. A missing key is never a crash and never a silent English fallback: it renders as a visible marker, and the tests
//! fail first for every shipped language.
//!
//! The words are the project's plain words (`docs/glossary.md`): *proposal*, *revision*, *held*, *trust a key* — never *seal*,
//! *adopt* or *intent*. Retired words and placeholder agreement are tests (`tests/catalog.rs`).

pub mod en;
pub mod ja;

pub use planeter_i18n::{Entry, LangDef, msg};

/// This program's catalog of every language in the registry (`planeter_i18n::languages::LANGUAGES`), in the registry's order. A test
/// fails if a registered language has no catalog here or the other way round. Whether a language is shipped, a draft or signed off is
/// data on the language, not a flag here.
pub static CATALOGS: [&LangDef; 2] = [&en::DEF, &ja::DEF];

/// A language of the catalog: a handle on the language and this program's messages for it.
#[derive(Clone, Copy)]
pub struct Lang(&'static LangDef);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.tag)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.tag == other.0.tag
    }
}

impl Eq for Lang {}

impl Lang {
    /// The language the messages are written in, and the fallback.
    pub const SOURCE: Lang = Lang(&en::DEF);

    /// Every language this program has a catalog for that is shipped (the source language and the signed-off ones).
    pub fn shipped() -> Vec<Lang> {
        CATALOGS
            .iter()
            .copied()
            .filter(|d| d.is_shipped())
            .map(Lang)
            .collect()
    }

    /// Every catalog, drafts included.
    pub fn all() -> Vec<Lang> {
        CATALOGS.iter().copied().map(Lang).collect()
    }

    pub fn tag(self) -> &'static str {
        self.0.tag
    }

    pub fn def(self) -> &'static LangDef {
        self.0
    }

    pub fn entries(self) -> &'static [Entry] {
        self.0.entries
    }

    /// The shipped tags, `en|ja`: what `--help` names and the refusal of a wrong `--lang` lists.
    pub fn shipped_tags(separator: &str) -> String {
        Lang::shipped()
            .iter()
            .map(|l| l.tag())
            .collect::<Vec<_>>()
            .join(separator)
    }

    fn shipped_languages() -> Vec<&'static planeter_i18n::Language> {
        Lang::shipped().iter().map(|l| l.0.lang).collect()
    }

    fn from_language(found: &planeter_i18n::Language) -> Option<Lang> {
        Lang::shipped()
            .into_iter()
            .find(|l| std::ptr::eq(l.0.lang, found))
    }

    /// The language a person asked for with `--lang`: any shipped tag, matched as a BCP 47 tag (`ja`, `ja-JP`, `EN`).
    pub fn from_flag(tag: &str) -> Option<Lang> {
        let found = planeter_i18n::tag::matching(tag.trim(), &Lang::shipped_languages())?;
        Lang::from_language(found)
    }

    /// The language to speak when no flag was given: the language the first of `LC_ALL`, `LC_MESSAGES`, `LANG` that is set and not
    /// empty names (`de_DE.UTF-8`, `zh_TW.Big5`; `C` and `POSIX` name none), among the shipped languages; the source language
    /// otherwise.
    pub fn from_environment(var: impl Fn(&str) -> Option<String>) -> Lang {
        planeter_i18n::tag::from_environment(var, &Lang::shipped_languages())
            .and_then(Lang::from_language)
            .unwrap_or(Lang::SOURCE)
    }
}

macro_rules! keys {
    ($($variant:ident => $id:literal,)*) => {
        /// The keys. A handler that names a key that does not exist does not compile.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Key {
            $($variant,)*
        }

        impl Key {
            /// Every key, in declaration order.
            pub const ALL: &'static [Key] = &[$(Key::$variant,)*];

            /// The identifier the catalogs and the tests use.
            pub const fn id(self) -> &'static str {
                match self {
                    $(Key::$variant => $id,)*
                }
            }
        }
    };
}

keys! {
    // usage and help
    HelpMain => "help.main",
    HelpPropose => "help.propose",
    HelpClone => "help.clone",
    HelpFetch => "help.fetch",
    VersionLine => "version.line",
    UsageUnknownCommand => "usage.unknown_command",
    UsageUnknownOption => "usage.unknown_option",
    UsageMissingValue => "usage.missing_value",
    UsageMissingArgument => "usage.missing_argument",
    UsageTooMany => "usage.too_many",
    UsageBadLang => "usage.bad_lang",
    UsageBadUrl => "usage.bad_url",
    UsagePlainHttp => "usage.plain_http",
    UsageNoRepository => "usage.no_repository",
    // the token
    TokenRequired => "token.required",
    TokenFileUnreadable => "token.file_unreadable",
    TokenFileTooOpen => "token.file_too_open",
    TokenFileEmpty => "token.file_empty",
    // the network and TLS
    TlsUnknownIssuer => "tls.unknown_issuer",
    TlsWrongName => "tls.wrong_name",
    TlsExpired => "tls.expired",
    TlsOther => "tls.other",
    NetUnreachable => "net.unreachable",
    NetTimeout => "net.timeout",
    NetCutShort => "net.cut_short",
    NetRedirect => "net.redirect",
    NetTooLong => "net.too_long",
    NetOther => "net.other",
    NetBadAnswer => "net.bad_answer",
    NetCacertUnreadable => "net.cacert_unreadable",
    NetProxy => "net.proxy",
    // what the server said
    ServerRefused => "server.refused",
    ServerRefusedPlain => "server.refused_plain",
    ServerNotFound => "server.not_found",
    ServerForbidden => "server.forbidden",
    ServerSignIn => "server.sign_in",
    ServerRateLimited => "server.rate_limited",
    ServerBusy => "server.busy",
    ServerOff => "server.off",
    ServerTooLarge => "server.too_large",
    ServerUnavailable => "server.unavailable",
    // propose
    ProposeStepAdvertise => "propose.step.advertise",
    ProposeStepHave => "propose.step.have",
    ProposeStepBuild => "propose.step.build",
    ProposeStepDeclare => "propose.step.declare",
    ProposeStepSend => "propose.step.send",
    ProposeDoneNew => "propose.done_new",
    ProposeDoneRevision => "propose.done_revision",
    ProposeAlready => "propose.already",
    ProposeNothingNew => "propose.nothing_new",
    ProposeDryRun => "propose.dry_run",
    ProposeDryRunDetails => "propose.dry_run_details",
    ProposeNoHave => "propose.no_have",
    ProposeBranchGone => "propose.branch_gone",
    ProposeBaseNotInHistory => "propose.base_not_in_history",
    ProposeOverBound => "propose.over_bound",
    ProposeTooManyObjects => "propose.too_many_objects",
    ProposeTitleNeeded => "propose.title_needed",
    ProposeIncomplete => "propose.incomplete",
    ProposeIncompleteRetry => "propose.incomplete_retry",
    ProposeQuota => "propose.quota",
    ProposeQuotaRemedy => "propose.quota_remedy",
    ProposeNeedsRecovery => "propose.needs_recovery",
    ProposeDeclaration => "propose.declaration",
    ProposeBaseUnknown => "propose.base_unknown",
    ProposeTooLargeForServer => "propose.too_large_for_server",
    ProposeNotOpen => "propose.not_open",
    ProposeNotYours => "propose.not_yours",
    ProposeUnknownProposal => "propose.unknown_proposal",
    ProposeNoPatches => "propose.no_patches",
    ProposeAcceptRefused => "propose.accept_refused",
    // clone
    CloneStepDownload => "clone.step.download",
    CloneStepVerify => "clone.step.verify",
    CloneStepImport => "clone.step.import",
    CloneKeysHeader => "clone.keys.header",
    CloneKeysNone => "clone.keys.none",
    CloneKeyShow => "clone.key.show",
    CloneKeyAsk => "clone.key.ask",
    CloneKeyTrusted => "clone.key.trusted",
    CloneKeySkipped => "clone.key.skipped",
    CloneKeyWrong => "clone.key.wrong",
    CloneKeyNotCarried => "clone.key.not_carried",
    CloneNoTerminal => "clone.no_terminal",
    CloneNoAdopt => "clone.no_adopt",
    CloneDone => "clone.done",
    CloneDoneNext => "clone.done_next",
    CloneDirExists => "clone.dir_exists",
    CloneVerifyFailed => "clone.verify_failed",
    CloneArchiveBad => "clone.archive_bad",
    // fetch
    FetchStepAsk => "fetch.step.ask",
    FetchStepDownload => "fetch.step.download",
    FetchStepImport => "fetch.step.import",
    FetchUnchanged => "fetch.unchanged",
    FetchDone => "fetch.done",
    FetchDoneNext => "fetch.done_next",
    FetchNotARepository => "fetch.not_a_repository",
    FetchSplitView => "fetch.split_view",
    FetchNoSequence => "fetch.no_sequence",
    FetchOlder => "fetch.older_state",
    FetchSameNumber => "fetch.same_number",
    // prikk and local state
    PrikkMissing => "prikk.missing",
    PrikkTooOld => "prikk.too_old",
    PrikkCannotStart => "prikk.cannot_start",
    PrikkNoVersion => "prikk.no_version",
    PrikkFailed => "prikk.failed",
    LocalFile => "local.file",
    LocalState => "local.state",
    // progress
    ProgressDownloaded => "progress.downloaded",
    ProgressSent => "progress.sent",
}

/// The messages of a signed-off language as they were signed (`locales/<tag>.signed`: `key<TAB>hash`), for the review baseline.
pub fn baseline_text(tag: &str) -> Option<&'static str> {
    match tag {
        "ja" => Some(include_str!("../locales/ja.signed")),
        _ => None,
    }
}

/// How many messages of `tag` are changed or new since its sign-off (`None` for a language that is not signed off).
pub fn unreviewed_count(tag: &str) -> Option<usize> {
    let def = CATALOGS
        .iter()
        .find(|d| d.tag == tag && d.sign_off().is_some())?;
    let baseline = planeter_i18n::baseline::parse(baseline_text(tag)?).ok()?;
    Some(planeter_i18n::baseline::unreviewed_count(
        def.entries,
        &baseline,
    ))
}

/// The text of `key` in `lang`, or `None` if the table lacks it.
pub fn raw(lang: Lang, key: Key) -> Option<&'static str> {
    lang.entries()
        .iter()
        .find(|e| e.key == key.id())
        .map(|e| e.other())
}

/// A message being filled in.
#[derive(Debug, Clone)]
pub struct Msg {
    lang: Lang,
    key: Key,
    args: Vec<(&'static str, String)>,
}

/// Start a message.
pub fn t(lang: Lang, key: Key) -> Msg {
    Msg {
        lang,
        key,
        args: Vec::new(),
    }
}

impl Msg {
    /// Fill the placeholder `{name}`.
    pub fn with(mut self, name: &'static str, value: impl std::fmt::Display) -> Msg {
        self.args.push((name, value.to_string()));
        self
    }

    /// The sentence. A missing key is a visible marker, never a silent fallback.
    pub fn text(&self) -> String {
        let Some(template) = raw(self.lang, self.key) else {
            return format!("⟦{}⟧", self.key.id());
        };
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after.find('}') {
                Some(close) => {
                    let name = &after[..close];
                    match self.args.iter().find(|(n, _)| *n == name) {
                        Some((_, v)) => out.push_str(v),
                        None => {
                            out.push('{');
                            out.push_str(name);
                            out.push('}');
                        }
                    }
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// The placeholder names a template uses, in order of appearance (for the tests).
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        found.push(&after[..close]);
        rest = &after[close + 1..];
    }
    found
}

/// A size for a person: bytes, KiB, MiB or GiB, one decimal place from KiB up.
pub fn size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else if b < KIB * KIB * KIB {
        format!("{:.1} MiB", b / (KIB * KIB))
    } else {
        format!("{:.1} GiB", b / (KIB * KIB * KIB))
    }
}
