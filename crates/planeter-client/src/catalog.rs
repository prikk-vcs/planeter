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

/// Whether the Japanese catalog has been signed off by its reviewers (the owner and a colleague; RFC 012 D-3f; the record is the
/// server's `JA_SIGNOFF`). Japanese is chosen from `LC_ALL`, `LC_MESSAGES` or `LANG` when they say `ja`; `--lang` always wins.
pub const JA_REVIEWED: bool = true;

/// A language of the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ja];

    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ja => "ja",
        }
    }

    pub fn entries(self) -> &'static [Entry] {
        match self {
            Lang::En => en::CATALOG,
            Lang::Ja => ja::CATALOG,
        }
    }

    /// The language a person asked for with `--lang`. Any known tag (English, Japanese).
    pub fn from_flag(tag: &str) -> Option<Lang> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "ja" => Some(Lang::Ja),
            _ => None,
        }
    }

    /// The language to speak when no flag was given: the first of `LC_ALL`, `LC_MESSAGES`, `LANG` that is set and not empty
    /// (the usual order), by its language part. English unless that is a *shipped* language.
    pub fn from_environment(var: impl Fn(&str) -> Option<String>) -> Lang {
        let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| var(name))
            .find(|v| !v.trim().is_empty());
        let Some(value) = value else {
            return Lang::En;
        };
        let primary = value
            .split(['_', '.', '@', '-'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match primary.as_str() {
            "ja" if JA_REVIEWED => Lang::Ja,
            _ => Lang::En,
        }
    }
}

/// One message: the key's identifier and its text.
#[derive(Debug)]
pub struct Entry {
    pub key: &'static str,
    pub text: &'static str,
}

/// A message.
pub const fn msg(key: &'static str, text: &'static str) -> Entry {
    Entry { key, text }
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

/// The text of `key` in `lang`, or `None` if the table lacks it.
pub fn raw(lang: Lang, key: Key) -> Option<&'static str> {
    lang.entries()
        .iter()
        .find(|e| e.key == key.id())
        .map(|e| e.text)
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
