//! The message catalog (RFC 012 D-3): every sentence the UI and the API's error bodies say, keyed by a
//! stable identifier, one compiled-in table per language, whole sentences with **named placeholders**.
//!
//! * **Keys** are an enum ([`Key`]): a template or handler that names a key that does not exist does not
//!   compile; the identifier string (`"login.failed"`) is what the catalogs and the tests use.
//! * **Languages** are [`LangDef`] values (tag, direction, plural rule, number format, table). English is
//!   [`EN`]; adding a language is a new table, a new `LangDef`, and a line in [`shipped`].
//! * **Placeholders** are `{name}`. A sentence that contains a link or emphasis takes a typed [`Slot`]:
//!   text parts are HTML-escaped, a link slot is built from an escaped label and an `href` the **code**
//!   supplies (never the catalog), and the result ([`Rich`]) is a type askama renders without a second
//!   escape. A catalog entry cannot introduce a tag, an attribute or a URL by itself: its own text is
//!   escaped like any other, and the tests refuse markup in it.
//! * **Plurals** follow the language's rule ([`Plural`]); a message with a count has a singular and an
//!   other form and takes `{count}`.
//! * **No concatenation**: a sentence is one message. Two sentences next to each other are two messages.
//! * A missing key is never a crash and never a silent English fallback: at run time it renders as a
//!   visible marker and is logged; the tests fail first for every shipped language.

use std::fmt;
use std::sync::{Mutex, OnceLock};

use askama::filters::HtmlSafe;

pub mod en;
pub mod ja;

/// The product name: the same in every language, so it is a value, not a message.
pub const PRODUCT: &str = "planeter";

/// Text direction of a language (`dir` on `<html>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Ltr,
    Rtl,
}

/// The plural categories this scheme knows. CLDR has more (`zero`, `two`, `few`, `many`); a language that
/// needs them is where the question of a fluent-style library is reopened (RFC 012 D-3). Callers use
/// [`Msg::n`] and never name a category, so adding one does not rewrite them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    One,
    Other,
}

/// One message: the key's identifier and its text for the singular and the other form. A message with no
/// count has `one == other`.
#[derive(Debug)]
pub struct Entry {
    pub key: &'static str,
    pub one: &'static str,
    pub other: &'static str,
}

/// A message with no plural forms.
pub const fn msg(key: &'static str, text: &'static str) -> Entry {
    Entry {
        key,
        one: text,
        other: text,
    }
}

/// A message with a singular and an other form (it must contain `{count}`).
pub const fn plural_msg(key: &'static str, one: &'static str, other: &'static str) -> Entry {
    Entry { key, one, other }
}

/// Everything that makes a language: how to name it, write it, count in it and where its messages are.
#[derive(Debug)]
pub struct LangDef {
    /// The BCP 47 primary tag (`en`), also the cookie value and `<html lang>`.
    pub tag: &'static str,
    /// The language's name in itself (`English`), for the switcher.
    pub endonym: &'static str,
    pub dir: Dir,
    pub plural: fn(u64) -> Plural,
    pub group_separator: char,
    pub decimal_separator: char,
    pub entries: &'static [Entry],
}

/// English: `one` for exactly 1, `other` for everything else.
fn plural_en(n: u64) -> Plural {
    if n == 1 { Plural::One } else { Plural::Other }
}

/// English, the default and the fallback for negotiation.
pub static EN_DEF: LangDef = LangDef {
    tag: "en",
    endonym: "English",
    dir: Dir::Ltr,
    plural: plural_en,
    group_separator: ',',
    decimal_separator: '.',
    entries: en::CATALOG,
};

/// A language handle: cheap to copy, carried by every page.
#[derive(Clone, Copy)]
pub struct Lang(&'static LangDef);

impl fmt::Debug for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lang({})", self.0.tag)
    }
}
impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.tag == other.0.tag
    }
}
impl Eq for Lang {}

/// English.
pub const EN: Lang = Lang(&EN_DEF);

/// Japanese has no grammatical plural: every count takes the `other` form.
fn plural_ja(_: u64) -> Plural {
    Plural::Other
}

/// Japanese (RFC 012 D-3, handoff `japanese-and-switcher-handoff-v1`): **a draft for the reviewers**. Compiled
/// in, checked by the tests, and **not shipped** until [`JA_REVIEWED`] is `true`.
pub static JA_DEF: LangDef = LangDef {
    tag: "ja",
    endonym: "日本語",
    dir: Dir::Ltr,
    plural: plural_ja,
    group_separator: ',',
    decimal_separator: '.',
    entries: ja::CATALOG,
};

/// Whether the Japanese catalog has been **signed off** by its reviewers (the owner and a colleague; RFC 012
/// D-3f). `false` until the reviewed sheet comes back and phase 2 integrates it; `shipped()` lists Japanese only
/// when this is `true`. A test pins both halves of the rule.
pub const JA_REVIEWED: bool = false;

/// The record of the sign-off: who reviewed the Japanese catalog and when, set together with [`JA_REVIEWED`]
/// (a test refuses one without the other). `None` while the catalog is a draft.
pub const JA_SIGNOFF: Option<&str> = None;

/// The languages this build ships. (Only these are checked for completeness; a language that is not signed off
/// is not here.)
pub fn shipped() -> &'static [&'static LangDef] {
    static WITH_JA: [&LangDef; 2] = [&EN_DEF, &JA_DEF];
    static WITHOUT_JA: [&LangDef; 1] = [&EN_DEF];
    if JA_REVIEWED { &WITH_JA } else { &WITHOUT_JA }
}

/// The languages that exist as drafts and can be previewed by the operator
/// (`PLANETER_PREVIEW_LANGUAGES`, handoff T7): compiled in, checked, but not signed off.
pub fn previewable() -> &'static [&'static LangDef] {
    static DRAFTS: [&LangDef; 1] = [&JA_DEF];
    &DRAFTS
}

/// Turn on the preview of draft languages for this process: `spec` is a comma-separated list of language codes
/// (`ja`); an empty value changes nothing. Returns the codes now previewable. An unknown code is an error that
/// names the ones that exist, so a typo does not silently do nothing.
pub fn enable_preview_languages(spec: &str) -> Result<Vec<&'static str>, String> {
    let mut enabled = Vec::new();
    for tag in spec.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match previewable()
            .iter()
            .find(|d| d.tag.eq_ignore_ascii_case(tag))
        {
            Some(def) => {
                register_test_language(def);
                enabled.push(def.tag);
            }
            None => {
                let known: Vec<&str> = previewable().iter().map(|d| d.tag).collect();
                return Err(format!(
                    "unknown language {tag:?}; the languages that can be previewed are: {}",
                    known.join(", ")
                ));
            }
        }
    }
    Ok(enabled)
}

static EXTRA: OnceLock<Mutex<Vec<&'static LangDef>>> = OnceLock::new();

/// A test seam: make one more language negotiable in this process. Nothing in the server calls it; the
/// tests register a second catalog to prove negotiation, the cookie, `lang`/`dir`, `Vary`, plurals and
/// placeholders without shipping a second language. Registered languages are **not** "shipped": the
/// completeness checks do not see them.
#[doc(hidden)]
pub fn register_test_language(def: &'static LangDef) {
    let mut v = EXTRA.get_or_init(Default::default).lock().unwrap();
    if !v.iter().any(|d| d.tag == def.tag) {
        v.push(def);
    }
}

fn available() -> Vec<&'static LangDef> {
    let mut v: Vec<&'static LangDef> = shipped().to_vec();
    if let Some(extra) = EXTRA.get() {
        v.extend(extra.lock().unwrap().iter());
    }
    v
}

/// One entry of the language switcher: the code, the language's name in itself, and whether it is the page's.
#[derive(Clone, Debug)]
pub struct LangChoice {
    pub tag: &'static str,
    pub endonym: &'static str,
    pub current: bool,
}

impl Lang {
    pub fn def(self) -> &'static LangDef {
        self.0
    }
    pub fn tag(self) -> &'static str {
        self.0.tag
    }
    pub fn dir(self) -> &'static str {
        match self.0.dir {
            Dir::Ltr => "ltr",
            Dir::Rtl => "rtl",
        }
    }
    pub fn from_def(def: &'static LangDef) -> Lang {
        Lang(def)
    }

    /// The language for a language code (`ja`, `ja-JP`): the primary subtag decides.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let primary = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        available().into_iter().find(|d| d.tag == primary).map(Lang)
    }

    /// Negotiate the page language (RFC 012 D-3b): the `lang` cookie when it names an available language,
    /// else the best `Accept-Language` entry (q-values, `q=0` refused), else English.
    pub fn negotiate(accept_language: Option<&str>, cookie: Option<&str>) -> Lang {
        if let Some(l) = cookie.and_then(Lang::from_tag) {
            return l;
        }
        let mut best: Option<(f32, usize, Lang)> = None;
        for (i, part) in accept_language.unwrap_or("").split(',').enumerate() {
            let mut it = part.trim().split(';');
            let tag = it.next().unwrap_or("").trim();
            let q = it
                .find_map(|p| {
                    p.trim()
                        .strip_prefix("q=")
                        .and_then(|v| v.parse::<f32>().ok())
                })
                .unwrap_or(1.0);
            if q <= 0.0 || tag == "*" {
                continue;
            }
            if let Some(l) = Lang::from_tag(tag)
                && best.is_none_or(|(bq, bi, _)| q > bq || (q == bq && i < bi))
            {
                best = Some((q, i, l));
            }
        }
        best.map_or(EN, |(_, _, l)| l)
    }

    fn entry(self, key: Key) -> Option<&'static Entry> {
        self.0.entries.iter().find(|e| e.key == key.id())
    }

    /// A number with the language's digit grouping.
    pub fn number(self, n: u64) -> String {
        let s = n.to_string();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(self.0.group_separator);
            }
            out.push(c);
        }
        out
    }
}

macro_rules! keys {
    ($($variant:ident => $id:literal,)*) => {
        /// A message key. The enum is the compile-time check: naming a key that does not exist does not
        /// compile; the identifier is what the catalogs and the tests use.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Key { $($variant),* }
        impl Key {
            /// Every key, in declaration order.
            pub const ALL: &'static [Key] = &[$(Key::$variant),*];
            /// The stable identifier.
            pub fn id(self) -> &'static str { match self { $(Key::$variant => $id),* } }
            /// The variant's name, as written in templates and Rust (`Key::LoginFailed` → `LoginFailed`).
            pub fn variant(self) -> &'static str { match self { $(Key::$variant => stringify!($variant)),* } }
        }
    };
}

keys! {
    // layout and navigation
    LayoutSignIn => "layout.sign_in",
    LayoutSignOut => "layout.sign_out",
    LayoutSkip => "layout.skip",
    LayoutNavRepository => "layout.nav_repository",
    LayoutFooter => "layout.footer",
    LayoutLanguage => "layout.language",
    NavCode => "nav.code",
    NavHistory => "nav.history",
    NavRefs => "nav.refs",
    NavVerify => "nav.verify",
    // page titles (one pattern: page · context · product)
    TitleHome => "title.home",
    TitleHomeError => "title.home_error",
    TitleBusy => "title.busy",
    TitleLogin => "title.login",
    TitleLoginFailed => "title.login_failed",
    TitleLoginLocked => "title.login_locked",
    TitleNotFound => "title.not_found",
    TitleUnavailable => "title.unavailable",
    TitleTooLarge => "title.too_large",
    TitleRepo => "title.repo",
    TitleTree => "title.tree",
    TitleFile => "title.file",
    TitleHistory => "title.history",
    TitleChange => "title.change",
    TitleRefs => "title.refs",
    TitleVerify => "title.verify",
    // home
    HomeIntro => "home.intro",
    HomeAddressExample => "home.address_example",
    HomeAddressLabel => "home.address_label",
    HomeAddressError => "home.address_error",
    HomeOpen => "home.open",
    // sign-in
    LoginHeading => "login.heading",
    LoginFailed => "login.failed",
    LoginSso => "login.sso",
    LoginUser => "login.user",
    LoginPassword => "login.password",
    LoginSubmit => "login.submit",
    LoginLocked => "login.locked",
    // the plain pages
    NotFoundHeading => "not_found.heading",
    NotFoundBody => "not_found.body",
    UnavailableHeading => "unavailable.heading",
    UnavailableBody => "unavailable.body",
    BusyHeading => "busy.heading",
    BusyBody => "busy.body",
    TooLargeHeading => "too_large.heading",
    TooLargeBody => "too_large.body",
    NoSuchPathBody => "no_such_path.body",
    NoSuchPathLink => "no_such_path.link",
    // repository, tree, file
    RepoLocation => "repo.location",
    RepoLocationBranch => "repo.location_branch",
    TreeAt => "tree.at",
    SizeAndEncoding => "listing.size_and_encoding",
    ListingCaption => "listing.caption",
    ListingName => "listing.name",
    ListingDetails => "listing.details",
    ListingFolder => "listing.folder",
    FileBinary => "file.binary",
    FileDownloadNamed => "file.download_named",
    // sizes
    SizeBytes => "size.bytes",
    SizeKib => "size.kib",
    SizeMib => "size.mib",
    SizeGib => "size.gib",
    // assurance labels (trust-bearing)
    AssuranceVerified => "assurance.verified",
    AssuranceSealed => "assurance.sealed",
    AssuranceQueued => "assurance.queued",
    AssuranceUnverified => "assurance.unverified",
    AssuranceVerifyUnavailable => "assurance.verify_unavailable",
    // in-place explanations of the labels above and of two terms (RFC 012 D-4)
    ExplainSummary => "explain.summary",
    ExplainVerified => "explain.verified",
    ExplainSigned => "explain.signed",
    ExplainQueued => "explain.queued",
    ExplainFailed => "explain.failed",
    ExplainFailedHere => "explain.failed_here",
    ExplainUnavailable => "explain.unavailable",
    ExplainPatch => "explain.patch",
    ExplainRollback => "explain.rollback",
    // history, change, refs, verify
    HistoryHeading => "history.heading",
    HistoryRollback => "history.rollback",
    HistoryPatchCount => "history.patch_count",
    HistoryEmpty => "history.empty",
    HistoryColChange => "history.col.change",
    HistoryColKind => "history.col.kind",
    HistoryColPatches => "history.col.patches",
    HistoryChangeFallback => "history.change_fallback",
    ChangeHeading => "change.heading",
    ChangePatch => "change.patch",
    ChangeColOperation => "change.col.operation",
    ChangeColPaths => "change.col.paths",
    RefsBranches => "refs.branches",
    RefsTags => "refs.tags",
    RefsCurrent => "refs.current",
    RefsClosed => "refs.closed",
    RefsNoTags => "refs.no_tags",
    RefsHeading => "refs.heading",
    RefsColBranch => "refs.col.branch",
    RefsColTag => "refs.col.tag",
    RefsColState => "refs.col.state",
    RefsColTarget => "refs.col.target",
    VerifyHeading => "verify.heading",
    VerifyIntro => "verify.intro",
    VerifyFindings => "verify.findings",
    VerifyStatusEvaluated => "verify.status.evaluated",
    VerifyStatusFailed => "verify.status.failed",
    VerifyStatusNotEvaluated => "verify.status.not_evaluated",
    VerifyStatusNotRun => "verify.status.not_run",
    VerifyStatusHalted => "verify.status.halted",
    VerifyStatusHaltedAfter => "verify.status.halted_after",
    VerifyColStage => "verify.col.stage",
    VerifyColStatus => "verify.col.status",
    VerifyColDetail => "verify.col.detail",
    // the API's error messages and hints (one message and one hint per code; the raw route's own hint)
    ErrNotFoundMessage => "error.not_found.message",
    ErrNotFoundHint => "error.not_found.hint",
    ErrUnavailableMessage => "error.unavailable.message",
    ErrUnavailableHint => "error.unavailable.hint",
    ErrBusyMessage => "error.busy.message",
    ErrBusyHint => "error.busy.hint",
    ErrInvalidRequestMessage => "error.invalid_request.message",
    ErrInvalidRequestHint => "error.invalid_request.hint",
    ErrMethodNotAllowedMessage => "error.method_not_allowed.message",
    ErrMethodNotAllowedHint => "error.method_not_allowed.hint",
    ErrRawElsewhereMessage => "error.raw_elsewhere.message",
    ErrRawElsewhereHint => "error.raw_elsewhere.hint",
    ErrTooLargeMessage => "error.too_large.message",
    ErrTooLargeHint => "error.too_large.hint",
    ErrTooLargeRawHint => "error.too_large.raw_hint",
    ErrInternalMessage => "error.internal.message",
    ErrInternalHint => "error.internal.hint",
    ErrTransportOffMessage => "error.transport_off.message",
    ErrTransportOffHint => "error.transport_off.hint",
    ErrUnknownRefMessage => "error.unknown_ref.message",
    ErrUnknownRefHint => "error.unknown_ref.hint",
    ErrUnknownKindMessage => "error.unknown_kind.message",
    ErrUnknownKindHint => "error.unknown_kind.hint",
    ErrBundleOfReceivedMessage => "error.bundle_of_received_ref.message",
    ErrBundleOfReceivedHint => "error.bundle_of_received_ref.hint",
    ErrNotVerifiedMessage => "error.not_verified.message",
    ErrNotVerifiedHint => "error.not_verified.hint",
    ErrExportFailedMessage => "error.export_failed.message",
    ErrExportFailedHint => "error.export_failed.hint",
    ErrExportDeadlineHint => "error.export_failed.deadline_hint",
    ErrExportCeilingHint => "error.export_failed.ceiling_hint",
    ErrSpoolTooSmallHint => "error.export_failed.spool_hint",
    ErrRateLimitedMessage => "error.rate_limited.message",
    ErrRateLimitedHint => "error.rate_limited.hint",
    ErrRangeMessage => "error.range_not_satisfiable.message",
    ErrRangeHint => "error.range_not_satisfiable.hint",
    ErrRefRequiredHint => "error.invalid_request.ref_hint",
    // quota refusals of a proposed change (RFC 005 D-9)
    QuotaOpenChangesMessage => "quota.open_changes.message",
    QuotaOpenChangesHint => "quota.open_changes.hint",
    QuotaAcceptedBytesMessage => "quota.accepted_bytes.message",
    QuotaAcceptedBytesHint => "quota.accepted_bytes.hint",
    QuotaRevisionsMessage => "quota.revisions.message",
    QuotaRevisionsHint => "quota.revisions.hint",
    QuotaCapacityMessage => "quota.capacity.message",
    QuotaCapacityHint => "quota.capacity.hint",
    // proposing a change (RFC 004 part 3b)
    ErrForbiddenMessage => "error.forbidden.message",
    ErrForbiddenHint => "error.forbidden.hint",
    ErrLengthRequiredMessage => "error.length_required.message",
    ErrLengthRequiredHint => "error.length_required.hint",
    ErrPayloadTooLargeMessage => "error.payload_too_large.message",
    ErrPayloadTooLargeHint => "error.payload_too_large.hint",
    ErrNeedsRecoveryMessage => "error.needs_recovery.message",
    ErrNeedsRecoveryHint => "error.needs_recovery.hint",
    ErrQuotaExceededMessage => "error.quota_exceeded.message",
    ErrQuotaExceededHint => "error.quota_exceeded.hint",
    ErrBaseUnknownMessage => "error.base_unknown.message",
    ErrBaseUnknownHint => "error.base_unknown.hint",
    ErrDeclarationMissesAddedMessage => "error.declaration_misses_added.message",
    ErrDeclarationMissesAddedHint => "error.declaration_misses_added.hint",
    ErrDeclaredNotHeldMessage => "error.declared_not_held.message",
    ErrDeclaredNotHeldHint => "error.declared_not_held.hint",
    ErrNoNewContentMessage => "error.no_new_content.message",
    ErrNoNewContentHint => "error.no_new_content.hint",
    ErrUnknownChangeMessage => "error.unknown_change.message",
    ErrUnknownChangeHint => "error.unknown_change.hint",
    ErrChangeNotOpenMessage => "error.change_not_open.message",
    ErrChangeNotOpenHint => "error.change_not_open.hint",
    ErrNotYourChangeMessage => "error.not_your_change.message",
    ErrNotYourChangeHint => "error.not_your_change.hint",
    ErrAcceptRefusedMessage => "error.accept_refused.message",
    ErrAcceptRefusedHint => "error.accept_refused.hint",
    ErrIntentPendingMessage => "error.intent_pending.message",
    ErrIntentPendingHint => "error.intent_pending.hint",
    ErrIncompleteMessage => "error.incomplete.message",
    ErrIncompleteHint => "error.incomplete.hint",
    ErrFramingHint => "error.invalid_request.framing_hint",
    ErrMetadataHint => "error.invalid_request.metadata_hint",
    ErrForbiddenAdminMessage => "error.forbidden_admin.message",
    ErrForbiddenAdminHint => "error.forbidden_admin.hint",
    ErrProposeOffMessage => "error.propose_off.message",
    ErrProposeOffHint => "error.propose_off.hint",
    ProposeHeld => "propose.held",
    ProposeAlreadyRecorded => "propose.already_recorded",
    // recovery (RFC 004 part 4)
    RecoveryCauseAcceptDeadline => "recovery.cause.accept_deadline",
    RecoveryCauseAcceptCeiling => "recovery.cause.accept_ceiling",
    RecoveryCauseAcceptFailed => "recovery.cause.accept_failed",
    RecoveryCauseAcceptUnreadable => "recovery.cause.accept_unreadable",
    RecoveryCauseAcceptNotRecorded => "recovery.cause.accept_not_recorded",
    RecoveryCauseServerStopped => "recovery.cause.server_stopped",
    RecoveryCauseOther => "recovery.cause.other",
    RecoveryBlockerCauseNotOurs => "recovery.blocker.cause_not_ours",
    RecoveryBlockerLeaseHeld => "recovery.blocker.lease_held",
    RecoveryBlockerForeignProcess => "recovery.blocker.foreign_process",
    RecoveryBlockerDoctorProblems => "recovery.blocker.doctor_problems",
    RecoveryBlockerOutputUnreadable => "recovery.blocker.output_unreadable",
    RecoveryBlockerUnexpectedLock => "recovery.blocker.unexpected_lock",
    RecoveryBlockerTooManyOpenIntents => "recovery.blocker.too_many_open_intents",
    RecoveryBlockerProcessesUninspected => "recovery.blocker.processes_uninspected",
    RecoveryOutcomeRecovered => "recovery.outcome.recovered",
    RecoveryOutcomeStopped => "recovery.outcome.stopped",
    RecoveryOutcomeNotNow => "recovery.outcome.not_now",
    RecoveryOutcomeNothingToDo => "recovery.outcome.nothing_to_do",
    ErrUnknownIntentMessage => "error.unknown_intent.message",
    ErrUnknownIntentHint => "error.unknown_intent.hint",
    // pages that used to be bare strings (RFC 012 T7)
    PageLinkSignIn => "page.link.sign_in",
    PageLinkHome => "page.link.home",
    PageTemplateError => "page.template_error",
    PageCsrfTitle => "page.csrf.title",
    PageCsrfHeading => "page.csrf.heading",
    PageCsrfBody => "page.csrf.body",
    PageFormTitle => "page.form_incomplete.title",
    PageFormHeading => "page.form_incomplete.heading",
    PageFormBody => "page.form_incomplete.body",
    PageSsoOffTitle => "page.sso_off.title",
    PageSsoOffHeading => "page.sso_off.heading",
    PageSsoOffBody => "page.sso_off.body",
    PageSsoDownTitle => "page.sso_down.title",
    PageSsoDownHeading => "page.sso_down.heading",
    PageSsoDownBody => "page.sso_down.body",
    PageSsoIncompleteTitle => "page.sso_incomplete.title",
    PageSsoIncompleteHeading => "page.sso_incomplete.heading",
    PageSsoIncompleteBody => "page.sso_incomplete.body",
    PageSsoMismatchTitle => "page.sso_mismatch.title",
    PageSsoMismatchHeading => "page.sso_mismatch.heading",
    PageSsoMismatchBody => "page.sso_mismatch.body",
    PageSsoExpiredTitle => "page.sso_expired.title",
    PageSsoExpiredHeading => "page.sso_expired.heading",
    PageSsoExpiredBody => "page.sso_expired.body",
    PageSsoRefusedTitle => "page.sso_refused.title",
    PageSsoRefusedHeading => "page.sso_refused.heading",
    PageSsoRefusedBody => "page.sso_refused.body",
    PageSsoUnlinkedTitle => "page.sso_unlinked.title",
    PageSsoUnlinkedHeading => "page.sso_unlinked.heading",
    PageSsoUnlinkedBody => "page.sso_unlinked.body",
    PageServerErrorTitle => "page.server_error.title",
    PageServerErrorHeading => "page.server_error.heading",
    PageServerErrorBody => "page.server_error.body",
}

/// Marks a response whose text depends on the caller's language, so that the response-policy layer adds
/// `Vary: Accept-Language` and `Content-Language` (a response extension).
#[derive(Clone, Copy, Debug)]
pub struct LanguageVaries;

/// The cookie that carries the visitor's language code (and nothing else): `SameSite=Lax`, not `HttpOnly`
/// (nothing secret), a year long. The footer switcher (`POST /lang`) sets it.
pub const LANG_COOKIE: &str = "planeter_lang";

/// A `Set-Cookie` value for the language cookie.
pub fn lang_cookie(tag: &str) -> String {
    format!("{LANG_COOKIE}={tag}; Path=/; Max-Age=31536000; Secure; SameSite=Lax")
}

/// The page language for a request: the language cookie, else `Accept-Language`, else English.
pub fn negotiate_from_headers(headers: &axum::http::HeaderMap) -> Lang {
    let accept = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    let cookie = crate::cookies::get_cookie(headers, LANG_COOKIE);
    Lang::negotiate(accept, cookie.as_deref())
}

/// A typed value for a placeholder. Text, code and muted slots are HTML-escaped; a link is an escaped label
/// plus an `href` the **code** supplies (a path starting `/`, or an `http(s)` URL; anything else renders as
/// the label alone). There is no way to hand the catalog's output raw markup.
#[derive(Clone, Debug)]
pub enum Slot {
    Text(String),
    Code(String),
    Muted(String),
    Link { label: String, href: String },
}

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

fn href_ok(href: &str) -> bool {
    let local = href.starts_with('/') && !href.starts_with("//") && !href.starts_with("/\\");
    let remote = href.starts_with("https://") || href.starts_with("http://");
    (local || remote) && !href.chars().any(|c| c.is_control() || c.is_whitespace())
}

impl Slot {
    fn plain(&self) -> &str {
        match self {
            Slot::Text(s) | Slot::Code(s) | Slot::Muted(s) => s,
            Slot::Link { label, .. } => label,
        }
    }
    fn html_into(&self, out: &mut String) {
        match self {
            Slot::Text(s) => escape_into(out, s),
            Slot::Code(s) => {
                out.push_str("<code>");
                escape_into(out, s);
                out.push_str("</code>");
            }
            Slot::Muted(s) => {
                out.push_str("<span class=\"muted\">");
                escape_into(out, s);
                out.push_str("</span>");
            }
            Slot::Link { label, href } if href_ok(href) => {
                out.push_str("<a href=\"");
                escape_into(out, href);
                out.push_str("\">");
                escape_into(out, label);
                out.push_str("</a>");
            }
            Slot::Link { label, .. } => escape_into(out, label),
        }
    }
}

/// Markup this module built from escaped parts: askama renders it as is. Only [`Msg::html`] makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rich(String);

impl Rich {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for Rich {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl HtmlSafe for Rich {}

/// A message being filled in.
#[derive(Clone, Debug)]
pub struct Msg {
    lang: Lang,
    key: Key,
    args: Vec<(&'static str, Slot)>,
    count: Option<u64>,
}

impl Msg {
    /// A plain-text placeholder.
    #[must_use]
    pub fn with(mut self, name: &'static str, value: impl ToString) -> Msg {
        self.args.push((name, Slot::Text(value.to_string())));
        self
    }
    /// A placeholder shown as code.
    #[must_use]
    pub fn code(mut self, name: &'static str, value: impl ToString) -> Msg {
        self.args.push((name, Slot::Code(value.to_string())));
        self
    }
    /// A placeholder shown in the muted style.
    #[must_use]
    pub fn muted(mut self, name: &'static str, value: impl ToString) -> Msg {
        self.args.push((name, Slot::Muted(value.to_string())));
        self
    }
    /// A placeholder that is a link: the label is text, the `href` comes from code.
    #[must_use]
    pub fn link(mut self, name: &'static str, label: impl ToString, href: impl ToString) -> Msg {
        self.args.push((
            name,
            Slot::Link {
                label: label.to_string(),
                href: href.to_string(),
            },
        ));
        self
    }
    /// The product name as `{product}`.
    #[must_use]
    pub fn product(self) -> Msg {
        self.with("product", PRODUCT)
    }
    /// A count: picks the plural form for the language and fills `{count}` with the grouped number.
    #[must_use]
    pub fn n(mut self, n: u64) -> Msg {
        self.count = Some(n);
        self.args.push(("count", Slot::Text(self.lang.number(n))));
        self
    }

    fn template(&self) -> String {
        match self.lang.entry(self.key) {
            Some(e) => match self.count.map(|n| (self.lang.0.plural)(n)) {
                Some(Plural::One) => e.one.to_owned(),
                _ => e.other.to_owned(),
            },
            None => {
                // Never a crash, never a silent fallback to another language: a visible marker, logged.
                eprintln!(
                    "planeter: the message {} is missing in the {} catalog",
                    self.key.id(),
                    self.lang.tag()
                );
                format!("[{}]", self.key.id())
            }
        }
    }

    fn render(&self, html: bool) -> String {
        let text = self.template();
        let mut out = String::with_capacity(text.len() + 16);
        let mut rest = text.as_str();
        let literal = |out: &mut String, s: &str| {
            if html {
                escape_into(out, s)
            } else {
                out.push_str(s)
            }
        };
        while let Some(i) = rest.find('{') {
            literal(&mut out, &rest[..i]);
            let Some(j) = rest[i..].find('}') else {
                literal(&mut out, &rest[i..]);
                rest = "";
                break;
            };
            let name = &rest[i + 1..i + j];
            match self.args.iter().find(|(k, _)| *k == name) {
                Some((_, slot)) if html => slot.html_into(&mut out),
                Some((_, slot)) => out.push_str(slot.plain()),
                // An unfilled placeholder stays visible (and the tests catch it).
                None => literal(&mut out, &rest[i..=i + j]),
            }
            rest = &rest[i + j + 1..];
        }
        literal(&mut out, rest);
        out
    }

    /// The message as plain text (API bodies, `<title>`, attributes).
    pub fn text(&self) -> String {
        self.render(false)
    }

    /// The message as HTML made of escaped parts and the typed slots.
    pub fn html(&self) -> Rich {
        Rich(self.render(true))
    }
}

impl fmt::Display for Msg {
    /// Plain text: askama escapes it on output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// What a page holds: its language, and a builder per message.
#[derive(Clone, Copy, Debug)]
pub struct T {
    pub lang: Lang,
}

impl T {
    pub fn new(lang: Lang) -> T {
        T { lang }
    }
    /// The message `key` being filled in.
    pub fn msg(&self, key: Key) -> Msg {
        Msg {
            lang: self.lang,
            key,
            args: Vec::new(),
            count: None,
        }
    }
    /// A message with no placeholders, as plain text.
    pub fn get(&self, key: Key) -> String {
        self.msg(key).text()
    }
    /// A title: the message with `{product}` filled and extra named values.
    pub fn title(&self, key: Key, values: &[(&'static str, &str)]) -> String {
        let mut m = self.msg(key).product();
        for (k, v) in values {
            m = m.with(k, v);
        }
        m.text()
    }
    pub fn lang_tag(&self) -> &'static str {
        self.lang.tag()
    }
    /// The languages the switcher offers (every language this process can serve), the page's marked current.
    pub fn languages(&self) -> Vec<LangChoice> {
        available()
            .into_iter()
            .map(|d| LangChoice {
                tag: d.tag,
                endonym: d.endonym,
                current: d.tag == self.lang.tag(),
            })
            .collect()
    }
    /// Where the switcher sends the visitor back to: the path of the page being rendered (see
    /// [`crate::lang_switch`]), `/` when there is none.
    pub fn switch_back(&self) -> String {
        crate::lang_switch::back()
    }
    pub fn dir(&self) -> &'static str {
        self.lang.dir()
    }
    pub fn product(&self) -> &'static str {
        PRODUCT
    }
    /// The running version, for the page footer: a number, not a translatable word.
    pub fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    /// A size with the unit from the catalog (`223 B`, `2.0 KiB`, `5.0 MiB`).
    pub fn size(&self, bytes: u64) -> String {
        const KIB: u64 = 1024;
        let (key, divisor) = match bytes {
            0..KIB => {
                return self
                    .msg(Key::SizeBytes)
                    .with("value", self.lang.number(bytes))
                    .text();
            }
            b if b < KIB * KIB => (Key::SizeKib, KIB),
            b if b < KIB * KIB * KIB => (Key::SizeMib, KIB * KIB),
            _ => (Key::SizeGib, KIB * KIB * KIB),
        };
        let tenths = (bytes as u128 * 10 + divisor as u128 / 2) / divisor as u128;
        let value = format!(
            "{}{}{}",
            self.lang.number((tenths / 10) as u64),
            self.lang.0.decimal_separator,
            tenths % 10
        );
        self.msg(key).with("value", value).text()
    }
}

/// Every placeholder name in a message text, sorted and de-duplicated.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        match rest[i..].find('}') {
            Some(j) => {
                v.push(&rest[i + 1..i + j]);
                rest = &rest[i + j + 1..];
            }
            None => break,
        }
    }
    v.sort_unstable();
    v.dedup();
    v
}

/// One thing wrong with a set of catalogs (see [`audit`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    Missing {
        lang: &'static str,
        key: &'static str,
    },
    Orphan {
        lang: &'static str,
        key: &'static str,
    },
    Duplicate {
        lang: &'static str,
        key: &'static str,
    },
    PlaceholdersDiffer {
        key: &'static str,
        lang: &'static str,
    },
    PluralFormsDisagree {
        key: &'static str,
        lang: &'static str,
    },
    Markup {
        lang: &'static str,
        key: &'static str,
    },
}

/// The completeness and consistency checks of RFC 012 D-5 over `langs` (English first; the other
/// languages are compared with it): every key in every language, no key a catalog has and the enum does
/// not, no duplicates, the same placeholder names everywhere, plural forms that fit the language's rule,
/// and no markup, entity or URL in any message.
pub fn audit(langs: &[&'static LangDef]) -> Vec<Problem> {
    let mut problems = Vec::new();
    let base = langs.first().copied();
    for def in langs {
        let ids: Vec<&'static str> = def.entries.iter().map(|e| e.key).collect();
        for k in Key::ALL {
            if !ids.contains(&k.id()) {
                problems.push(Problem::Missing {
                    lang: def.tag,
                    key: k.id(),
                });
            }
        }
        for (i, e) in def.entries.iter().enumerate() {
            if !Key::ALL.iter().any(|k| k.id() == e.key) {
                problems.push(Problem::Orphan {
                    lang: def.tag,
                    key: e.key,
                });
            }
            if def.entries[..i].iter().any(|p| p.key == e.key) {
                problems.push(Problem::Duplicate {
                    lang: def.tag,
                    key: e.key,
                });
            }
            for text in [e.one, e.other] {
                if text.contains(['<', '>', '&'])
                    || text.contains("http://")
                    || text.contains("https://")
                {
                    problems.push(Problem::Markup {
                        lang: def.tag,
                        key: e.key,
                    });
                    break;
                }
            }
            // A message with a count must say both forms where the language has two; a language whose rule
            // has only `other` must not pretend to have two.
            let counted =
                placeholders(e.one).contains(&"count") || placeholders(e.other).contains(&"count");
            let has_one_form = (0..=2).any(|n| (def.plural)(n) == Plural::One);
            if (!counted && e.one != e.other)
                || (counted && has_one_form && e.one == e.other)
                || (counted && !has_one_form && e.one != e.other)
            {
                problems.push(Problem::PluralFormsDisagree {
                    key: e.key,
                    lang: def.tag,
                });
            }
            if let Some(b) = base.filter(|b| b.tag != def.tag)
                && let Some(be) = b.entries.iter().find(|x| x.key == e.key)
            {
                let mut want = placeholders(be.one);
                want.extend(placeholders(be.other));
                want.sort_unstable();
                want.dedup();
                let mut got = placeholders(e.one);
                got.extend(placeholders(e.other));
                got.sort_unstable();
                got.dedup();
                if want != got {
                    problems.push(Problem::PlaceholdersDiffer {
                        key: e.key,
                        lang: def.tag,
                    });
                }
            }
        }
    }
    problems
}
