#![forbid(unsafe_code)]
//! `planeter-i18n-sheet` — the review sheet for any language (RFC 012 D-3f, amended 2026-11-04; handoffs
//! `japanese-consolidated-sheet-handoff-v1` and `language-registry-handoff-v1`).
//!
//! The sheet is **generated** from the live catalogs of both programs (the server's and `planeter-client`'s), never written by hand:
//! every row's English and target-language cell is the catalog's text, so what is reviewed is exactly what is compiled. [`check_against`]
//! reads a sheet back and reports every way it can have drifted from the catalogs: a cell that differs, a message the sheet lacks, a key
//! the catalogs do not have, a row twice, a row in the wrong group, a stale status.
//!
//! Rows are grouped by what a reader sees (the reviewer may stop after any group): **A** the words that carry trust, **B** the pages a
//! person reads (page by page), **C** the command-line program, **D** developer- and operator-facing texts (optional for the sign-off).
//! The group, the page and the "where it appears" of a message come from `data/contexts.tsv` (the longest matching pattern; a message
//! that matches none is an error), the notes for the reviewer from `data/<tag>/notes.tsv`, the first-page instructions from
//! `data/<tag>/instructions`, and the hard terms from `docs/glossary/<tag>.md`.
//!
//! For a **signed-off** language every row also says where it stands against the sign-off — `unchanged`, `changed since sign-off` or
//! `new since sign-off` — from the baseline each program keeps (`locales/<tag>.signed`); a message written after the sign-off ships and
//! is flagged until a reviewer has read it.
//!
//! A message that is English-only (`ENGLISH_ONLY_KEYS` of the server's catalog) has no entry in the target language and is not in the sheet.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use planeter_i18n::baseline::{self, ReviewState};
use planeter_i18n::{Entry, Language, Status};

pub const CONTEXTS: &str = include_str!("../data/contexts.tsv");

/// The first-page instructions and the reviewers' notes of each language that has them: `(tag, instructions, notes)`. A language is
/// added with a line here and the two files.
pub const LANGUAGE_DATA: &[(&str, &str, &str)] = &[(
    "ja",
    include_str!("../data/ja/instructions"),
    include_str!("../data/ja/notes.tsv"),
)];

/// Which program a message belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Program {
    Web,
    Client,
}

impl Program {
    fn word(self) -> &'static str {
        match self {
            Program::Web => "web",
            Program::Client => "client",
        }
    }
}

/// The four groups of the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    A,
    B,
    C,
    D,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::A, Group::B, Group::C, Group::D];

    pub fn letter(self) -> char {
        match self {
            Group::A => 'A',
            Group::B => 'B',
            Group::C => 'C',
            Group::D => 'D',
        }
    }

    fn from_letter(s: &str) -> Option<Group> {
        match s {
            "A" => Some(Group::A),
            "B" => Some(Group::B),
            "C" => Some(Group::C),
            "D" => Some(Group::D),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Group::A => "The words that carry trust",
            Group::B => "The pages a person reads, page by page",
            Group::C => "The command-line program (planeter-client)",
            Group::D => "Developer- and operator-facing texts (optional for the sign-off)",
        }
    }

    pub fn covers(self) -> &'static str {
        match self {
            Group::A => {
                "the badges and their explanations, the review line and the decision words, the states of a proposal and the sentences about patches the server holds or cannot place, the screens where a person decides which maintainer keys to trust"
            }
            Group::B => {
                "home, repository, files, history, branches and tags, verification, signing in, proposals (the list and one proposal), review, comments, creating a repository, and the missing, unavailable and error pages, in the order a person meets them"
            }
            Group::C => {
                "the help, the sentences and the refusals of `planeter-client` (clone, fetch, propose)"
            }
            Group::D => {
                "API error messages and hints, quota refusals, recovery-plan sentences and the answer to a proposal; these are read by developers and operators, and the owner may decide to keep them in English (then this group is empty)"
            }
        }
    }
}

/// One row of the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub program: Program,
    pub key: String,
    pub group: Group,
    pub page: String,
    pub situation: String,
    /// The source-language cell and the target-language cell, as text (before the table escaping).
    pub english: String,
    pub target: String,
    pub placeholders: Vec<String>,
    pub note: String,
    /// Against the sign-off; `None` for a language that is not signed off.
    pub state: Option<ReviewState>,
}

/// Why a sheet or its inputs cannot be used.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A message that no line of `contexts.tsv` covers.
    NoContext { program: Program, key: String },
    /// `contexts.tsv` is malformed (a line, with the reason).
    BadContexts(String),
    /// A target-language message with no source message.
    NoEnglish { key: String },
    /// A note for a key that no catalog has.
    NoteForUnknownKey(String),
    /// `notes.tsv` is malformed.
    BadNotes(String),
    /// The language is not in the registry, or has no catalog in one of the programs.
    UnknownLanguage(String),
    /// The language's baseline is missing or malformed.
    BadBaseline(String),
    /// The language's instructions file is missing or malformed.
    BadInstructions(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoContext { program, key } => write!(
                f,
                "the {} message `{key}` matches no line of data/contexts.tsv (add one)",
                program.word()
            ),
            Error::BadContexts(m) => write!(f, "data/contexts.tsv: {m}"),
            Error::NoEnglish { key } => {
                write!(f, "the message `{key}` has no source-language message")
            }
            Error::NoteForUnknownKey(k) => {
                write!(f, "the notes have a note for `{k}`, which no catalog has")
            }
            Error::BadNotes(m) => write!(f, "notes.tsv: {m}"),
            Error::UnknownLanguage(t) => write!(
                f,
                "`{t}` is not a language of the registry with a catalog in both programs"
            ),
            Error::BadBaseline(m) => write!(f, "baseline: {m}"),
            Error::BadInstructions(m) => write!(f, "instructions: {m}"),
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------------------------------------- the maintained tables

/// One line of `contexts.tsv`.
#[derive(Clone, Debug)]
struct Context {
    program: Program,
    pattern: String,
    group: Group,
    page: String,
    situation: String,
}

/// The parsed `contexts.tsv`, with the order of the pages.
pub struct Contexts {
    lines: Vec<Context>,
}

impl Contexts {
    pub fn parse(text: &str) -> Result<Contexts, Error> {
        let mut lines = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |m: &str| Error::BadContexts(format!("line {}: {m}", n + 1));
            let cells: Vec<&str> = line.split('\t').collect();
            let [program, pattern, group, page, situation] = cells[..] else {
                return Err(at("expected five tab-separated columns"));
            };
            let program = match program {
                "web" => Program::Web,
                "client" => Program::Client,
                other => return Err(at(&format!("unknown program {other:?}"))),
            };
            let group =
                Group::from_letter(group).ok_or_else(|| at(&format!("unknown group {group:?}")))?;
            if pattern.is_empty()
                || pattern.matches('*').count() > 1
                || pattern.contains('*') && !pattern.ends_with('*')
            {
                return Err(at("a pattern is a key or a prefix ending in `*`"));
            }
            if page.is_empty() || situation.is_empty() {
                return Err(at("page and situation must not be empty"));
            }
            if lines
                .iter()
                .any(|c: &Context| c.program == program && c.pattern == pattern)
            {
                return Err(at(&format!("the pattern {pattern:?} is listed twice")));
            }
            lines.push(Context {
                program,
                pattern: pattern.to_owned(),
                group,
                page: page.to_owned(),
                situation: situation.to_owned(),
            });
        }
        // A page belongs to one group.
        let mut seen: BTreeMap<&str, Group> = BTreeMap::new();
        for c in &lines {
            if let Some(g) = seen.insert(&c.page, c.group)
                && g != c.group
            {
                return Err(Error::BadContexts(format!(
                    "the page {:?} is in two groups",
                    c.page
                )));
            }
        }
        Ok(Contexts { lines })
    }

    /// The line that covers `key`: an exact pattern beats any prefix, the longest prefix beats a shorter one.
    fn find(&self, program: Program, key: &str) -> Option<&Context> {
        let mut best: Option<(&Context, usize)> = None;
        for c in self.lines.iter().filter(|c| c.program == program) {
            let score = match c.pattern.strip_suffix('*') {
                Some(prefix) if key.starts_with(prefix) => prefix.len(),
                Some(_) => continue,
                None if c.pattern == key => usize::MAX,
                None => continue,
            };
            if best.is_none_or(|(_, s)| score > s) {
                best = Some((c, score));
            }
        }
        best.map(|(c, _)| c)
    }

    /// The position of a page in the sheet: where its first line stands.
    fn page_rank(&self, page: &str) -> usize {
        self.lines
            .iter()
            .position(|c| c.page == page)
            .unwrap_or(usize::MAX)
    }
}

/// The parsed notes file.
pub fn parse_notes(text: &str) -> Result<BTreeMap<String, String>, Error> {
    let mut notes = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, note)) = line.split_once('\t') else {
            return Err(Error::BadNotes(format!(
                "line {}: expected key<TAB>note",
                n + 1
            )));
        };
        if note.trim().is_empty() {
            return Err(Error::BadNotes(format!("line {}: an empty note", n + 1)));
        }
        if notes.insert(key.to_owned(), note.to_owned()).is_some() {
            return Err(Error::BadNotes(format!(
                "line {}: `{key}` has two notes",
                n + 1
            )));
        }
    }
    Ok(notes)
}

// ---------------------------------------------------------------------------------------------------- building the rows

/// The text of a message as one cell: the one text when it has only `other`, otherwise every form, labelled with its category.
pub fn message_cell(e: &Entry) -> String {
    let forms: Vec<_> = e.forms().collect();
    if forms.len() == 1 {
        return forms[0].1.to_owned();
    }
    forms
        .iter()
        .map(|(c, t)| format!("[{}] {t}", c.label()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The rows of one program for one language: every message of the target catalog (in its catalog order) with the source message
/// beside it. A key the target catalog does not hold (English-only) is not a row.
pub fn rows_of(
    program: Program,
    source: &[Entry],
    target: &[Entry],
    contexts: &Contexts,
    notes: &BTreeMap<String, String>,
    baseline: Option<&BTreeMap<String, String>>,
) -> Result<Vec<Row>, Error> {
    let states: BTreeMap<&str, ReviewState> = baseline
        .map(|b| baseline::review_states(target, b).into_iter().collect())
        .unwrap_or_default();
    let mut rows = Vec::new();
    for t in target {
        let Some(s) = source.iter().find(|e| e.key == t.key) else {
            return Err(Error::NoEnglish {
                key: t.key.to_owned(),
            });
        };
        let ctx = contexts
            .find(program, t.key)
            .ok_or_else(|| Error::NoContext {
                program,
                key: t.key.to_owned(),
            })?;
        let mut names: Vec<String> = Vec::new();
        for e in [s, t] {
            for (_, text) in e.forms() {
                for p in planeter_i18n::placeholders(text) {
                    if !names.iter().any(|n| n == p) {
                        names.push(p.to_owned());
                    }
                }
            }
        }
        rows.push(Row {
            program,
            key: t.key.to_owned(),
            group: ctx.group,
            page: ctx.page.clone(),
            situation: ctx.situation.clone(),
            english: message_cell(s),
            target: message_cell(t),
            placeholders: names,
            note: notes.get(t.key).cloned().unwrap_or_default(),
            state: baseline.map(|_| states.get(t.key).copied().unwrap_or(ReviewState::New)),
        });
    }
    Ok(rows)
}

/// Everything the sheet needs about a language from the two programs.
pub struct Sources<'a> {
    pub lang: &'a Language,
    pub web_source: &'a [Entry],
    pub web_target: &'a [Entry],
    pub client_source: &'a [Entry],
    pub client_target: &'a [Entry],
    pub web_baseline: Option<BTreeMap<String, String>>,
    pub client_baseline: Option<BTreeMap<String, String>>,
}

/// The sources of a registered language from the live catalogs of the two programs.
pub fn live_sources(tag: &str) -> Result<Sources<'static>, Error> {
    let unknown = || Error::UnknownLanguage(tag.to_owned());
    let lang = planeter_i18n::languages::by_tag(tag).ok_or_else(unknown)?;
    let web = |l: &Language| {
        planeter_web::catalog::CATALOGS
            .iter()
            .find(|d| std::ptr::eq(d.lang, l))
            .copied()
    };
    let cli = |l: &Language| {
        planeter_client::catalog::CATALOGS
            .iter()
            .find(|d| std::ptr::eq(d.lang, l))
            .copied()
    };
    let source = planeter_i18n::languages::source();
    let (web_src, web_tgt) = (
        web(source).ok_or_else(unknown)?,
        web(lang).ok_or_else(unknown)?,
    );
    let (cli_src, cli_tgt) = (
        cli(source).ok_or_else(unknown)?,
        cli(lang).ok_or_else(unknown)?,
    );
    let parse = |text: Option<&'static str>| -> Result<Option<BTreeMap<String, String>>, Error> {
        text.map(|t| baseline::parse(t).map_err(Error::BadBaseline))
            .transpose()
    };
    let (web_baseline, client_baseline) = if lang.sign_off().is_some() {
        let w = parse(planeter_web::catalog::baseline_text(tag))?
            .ok_or_else(|| Error::BadBaseline(format!("the server has no baseline for `{tag}`")))?;
        let c = parse(planeter_client::catalog::baseline_text(tag))?
            .ok_or_else(|| Error::BadBaseline(format!("the client has no baseline for `{tag}`")))?;
        (Some(w), Some(c))
    } else {
        (None, None)
    };
    Ok(Sources {
        lang,
        web_source: web_src.entries,
        web_target: web_tgt.entries,
        client_source: cli_src.entries,
        client_target: cli_tgt.entries,
        web_baseline,
        client_baseline,
    })
}

/// The notes of a language, or none if it has no notes file.
pub fn notes_of(tag: &str) -> Result<BTreeMap<String, String>, Error> {
    match LANGUAGE_DATA.iter().find(|(t, _, _)| *t == tag) {
        Some((_, _, notes)) => parse_notes(notes),
        None => Ok(BTreeMap::new()),
    }
}

/// Every row for a language, in the sheet's order: by group, then by page (in the order of `contexts.tsv`), and inside a page in
/// catalog order (the server's messages first, then the client's).
pub fn rows_for(
    src: &Sources<'_>,
    contexts: &Contexts,
    notes: &BTreeMap<String, String>,
) -> Result<Vec<Row>, Error> {
    let mut rows = rows_of(
        Program::Web,
        src.web_source,
        src.web_target,
        contexts,
        notes,
        src.web_baseline.as_ref(),
    )?;
    rows.extend(rows_of(
        Program::Client,
        src.client_source,
        src.client_target,
        contexts,
        notes,
        src.client_baseline.as_ref(),
    )?);
    // A note must belong to a key of the sheet's catalogs.
    let known: BTreeSet<&str> = src
        .web_source
        .iter()
        .chain(src.client_source)
        .map(|e| e.key)
        .collect();
    for key in notes.keys() {
        if !known.contains(key.as_str()) {
            return Err(Error::NoteForUnknownKey(key.clone()));
        }
    }
    rows.sort_by_key(|r| (r.group, contexts.page_rank(&r.page)));
    Ok(rows)
}

/// The rows of a registered language from the live catalogs.
pub fn live_rows(tag: &str) -> Result<Vec<Row>, Error> {
    let contexts = Contexts::parse(CONTEXTS)?;
    let notes = notes_of(tag)?;
    rows_for(&live_sources(tag)?, &contexts, &notes)
}

// ---------------------------------------------------------------------------------------------------- escaping a table cell

/// A text as the content of a Markdown table cell. Reversible by [`unescape_cell`]: backslash, bar, angle brackets, ampersand, emphasis
/// and code marks and brackets are escaped, a line break is `<br>`.
pub fn escape_cell(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\\' | '|' | '*' | '_' | '`' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\n' => out.push_str("<br>"),
            '\r' => out.push_str("&#13;"),
            other => out.push(other),
        }
    }
    out
}

/// The inverse of [`escape_cell`].
pub fn unescape_cell(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    let mut rest = cell;
    while let Some(c) = rest.chars().next() {
        if c == '\\' {
            let mut it = rest.chars();
            it.next();
            if let Some(n) = it.next() {
                out.push(n);
                rest = &rest[1 + n.len_utf8()..];
                continue;
            }
        }
        let table = [
            ("<br>", "\n"),
            ("&lt;", "<"),
            ("&gt;", ">"),
            ("&amp;", "&"),
            ("&#13;", "\r"),
        ];
        if let Some((from, to)) = table.iter().find(|(from, _)| rest.starts_with(from)) {
            out.push_str(to);
            rest = &rest[from.len()..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

// ---------------------------------------------------------------------------------------------------- the sheet

/// What the header of a sheet says about where it came from.
#[derive(Clone, Debug)]
pub struct Stamp {
    /// The commit the catalogs were compiled from (`unknown` if git said nothing), with a mark if the working tree was not clean.
    pub commit: String,
    pub dirty: bool,
    /// `YYYY-MM-DD`, UTC.
    pub date: String,
}

/// A term the reviewers decide first (a "Hard term" row of `docs/glossary/<tag>.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardTerm {
    pub english: String,
    pub target: String,
    pub why: String,
    pub status: String,
}

/// The hard terms of a language's glossary file (the rows of its "Terms" table whose status begins with "Hard term").
pub fn hard_terms(glossary: &str) -> Vec<HardTerm> {
    let mut on = false;
    let mut header_done = false;
    let mut out = Vec::new();
    for line in glossary.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            on = line.trim_start_matches('#').trim() == "Terms";
            header_done = false;
            continue;
        }
        if !on || !line.starts_with('|') {
            continue;
        }
        if line.contains("---") {
            header_done = true;
            continue;
        }
        if !header_done {
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split(" | ")
            .map(str::trim)
            .collect();
        if let [english, target, _shown, why, status] = cells[..]
            && status.starts_with("Hard term")
        {
            out.push(HardTerm {
                english: english.to_owned(),
                target: target.to_owned(),
                why: why.to_owned(),
                status: status.to_owned(),
            });
        }
    }
    out
}

/// FNV-1a over every row's program, key, source text and target text: an identifier for "this wording", for the record of a sign-off.
/// It changes when any shown text changes, not when a note or a status does; it is not a cryptographic hash.
pub fn fingerprint(rows: &[Row]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |s: &str| {
        for b in s.bytes().chain(std::iter::once(0u8)) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by(|a, b| (a.program, &a.key).cmp(&(b.program, &b.key)));
    for r in sorted {
        feed(r.program.word());
        feed(&r.key);
        feed(&r.english);
        feed(&r.target);
    }
    format!("{h:016x}")
}

/// The number of rows per group.
pub fn counts(rows: &[Row]) -> BTreeMap<Group, usize> {
    let mut m: BTreeMap<Group, usize> = Group::ALL.iter().map(|g| (*g, 0)).collect();
    for r in rows {
        *m.entry(r.group).or_default() += 1;
    }
    m
}

/// `(unchanged, changed, new)` against the sign-off (all zero for a language that is not signed off).
pub fn state_counts(rows: &[Row]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for r in rows {
        match r.state {
            Some(ReviewState::Unchanged) => c.0 += 1,
            Some(ReviewState::Changed) => c.1 += 1,
            Some(ReviewState::New) => c.2 += 1,
            None => {}
        }
    }
    c
}

/// The two sections of an instructions file, with the status-aware `{{draft|signed}}` and `{tag}`, `{endonym}` filled in.
pub fn instructions(text: &str, lang: &Language) -> Result<(String, String), Error> {
    let draft = !matches!(lang.status, Status::SignedOff(_));
    let mut english = String::new();
    let mut local = String::new();
    let mut into: Option<&mut String> = None;
    for line in text.lines() {
        match line.trim() {
            "=== english" => into = Some(&mut english),
            "=== local" => into = Some(&mut local),
            _ if line.starts_with('#') => {}
            _ => {
                if let Some(s) = into.as_deref_mut() {
                    s.push_str(line);
                    s.push('\n');
                }
            }
        }
    }
    if english.trim().is_empty() || local.trim().is_empty() {
        return Err(Error::BadInstructions(
            "both `=== english` and `=== local` sections are needed".to_owned(),
        ));
    }
    let fill = |s: String| -> Result<String, Error> {
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(i) = rest.find("{{") {
            let Some(j) = rest[i..].find("}}") else {
                return Err(Error::BadInstructions("an unclosed {{".to_owned()));
            };
            let inner = &rest[i + 2..i + j];
            let Some((a, b)) = inner.split_once('|') else {
                return Err(Error::BadInstructions(format!("{{{{{inner}}}}} needs a|b")));
            };
            out.push_str(&rest[..i]);
            out.push_str(if draft { a } else { b });
            rest = &rest[i + j + 2..];
        }
        out.push_str(rest);
        Ok(out
            .replace("{tag}", lang.tag)
            .replace("{endonym}", lang.endonym)
            .trim_end()
            .to_owned())
    };
    Ok((fill(english)?, fill(local)?))
}

/// The "what happens next" list, for a language and its status.
fn next_steps(lang: &Language) -> String {
    let tag = lang.tag;
    let signed = lang.sign_off().is_some();
    let mut s = format!(
        "1. Each group's corrections become edits of the `{tag}` catalogs (`crates/planeter-web/src/catalog/{tag}.rs`, `crates/planeter-client/src/catalog/{tag}.rs`) and of the language's glossary file (`docs/glossary/{tag}.md`) — **one commit per group**; a term decided in the table at the top is applied everywhere it is used, in the same commit that decides it.\n\
2. The sheet is regenerated after the last commit and checked (`planeter-i18n-sheet --lang {tag} --check`): every row says what the catalogs say.\n"
    );
    if signed {
        s.push_str(
            "3. A message written or changed **after** the sign-off ships and is flagged in the \"since sign-off\" column until a reviewer has read it; when the reviewers have read the flagged rows, the language's baseline files (`crates/*/locales/<tag>.signed`) are regenerated (`--baseline`) and its sign-off record in the registry (`planeter-i18n`, `languages/<tag>.rs`) gets a new line of record.\n",
        );
    } else {
        s.push_str(&format!(
            "3. The sign-off is recorded in the registry (`crates/planeter-i18n/src/languages/{tag}.rs`): the language's status becomes `SignedOff` with the reviewers' names or roles (at least one fluent reader), the date, the commit and this sheet's fingerprint, and the baseline files (`crates/*/locales/{tag}.signed`) are written (`--baseline`). A test refuses a sign-off without them.\n"
        ));
    }
    s.push_str(
        "4. A right-to-left language additionally needs a real page read by a fluent reader before it ships (`docs/messages-and-languages.md`, \"Adding a language\").\n\
5. From then on the next feature that adds a message regenerates this sheet; nobody writes an addendum again.",
    );
    s
}

fn column_header(signed: bool, endonym: &str, draft: bool) -> String {
    let target = if draft {
        format!("{endonym} draft")
    } else {
        format!("{endonym} (current text)")
    };
    if signed {
        format!(
            "| key | where it appears | English | {target} | placeholders | note for the reviewer | since sign-off | reviewer's correction |\n|---|---|---|---|---|---|---|---|"
        )
    } else {
        format!(
            "| key | where it appears | English | {target} | placeholders | note for the reviewer | reviewer's correction |\n|---|---|---|---|---|---|---|"
        )
    }
}

/// Render the whole sheet for one language.
pub fn render(
    rows: &[Row],
    lang: &Language,
    instructions: &(String, String),
    stamp: &Stamp,
    hard: &[HardTerm],
) -> String {
    let mut s = String::new();
    let counts = counts(rows);
    let total: usize = counts.values().sum();
    let signed = lang.sign_off().is_some();
    let mark = if stamp.dirty {
        " (+ uncommitted changes)"
    } else {
        ""
    };
    let _ = writeln!(
        s,
        "# {} review sheet — planeter (one sheet, generated)\n",
        lang.endonym
    );
    let _ = writeln!(
        s,
        "**Language `{}` ({}).** **Generated from the catalogs of commit `{}`{mark} on {}.** Catalog fingerprint `{}`. {total} messages. \
Regenerate with `cargo run -p planeter-i18n-sheet -- --lang {} --out FILE`; check a sheet against the catalogs with `cargo run -p planeter-i18n-sheet -- --lang {} --check FILE`. \
Do not edit the English, {} or key columns by hand: they are the catalogs' text, and the check fails on any difference.\n",
        lang.tag,
        match &lang.status {
            Status::Source => "source language",
            Status::Draft => "draft",
            Status::SignedOff(_) => "signed off",
        },
        stamp.commit,
        stamp.date,
        fingerprint(rows),
        lang.tag,
        lang.tag,
        lang.endonym,
    );
    if let Some(sign) = lang.sign_off() {
        let (unchanged, changed, new) = state_counts(rows);
        let _ = writeln!(
            s,
            "**Signed off** by {} on {} (commit `{}`, sheet fingerprint `{}`). **Since then: {unchanged} unchanged, {changed} changed, {new} new** — {} message(s) to read. A message written or changed after the sign-off ships and is flagged until a reviewer has read it.\n",
            sign.reviewers.join(", "),
            sign.date,
            sign.commit,
            sign.sheet_fingerprint,
            changed + new,
        );
    }
    let _ = writeln!(
        s,
        "**For:** the reviewers of this language. Nothing is published until you have gone through it and the corrections are integrated.\n"
    );
    let _ = writeln!(s, "## The groups (you may stop after any of them)\n");
    let _ = writeln!(s, "| group | rows | what it covers |\n|---|---|---|");
    for g in Group::ALL {
        let _ = writeln!(
            s,
            "| **{}. {}** | {} | {} |",
            g.letter(),
            g.title(),
            counts[&g],
            g.covers()
        );
    }
    let _ = writeln!(
        s,
        "\n## What to do ({})\n\n{}\n",
        lang.endonym, instructions.1
    );
    let _ = writeln!(s, "## What to do (English)\n\n{}\n", instructions.0);
    let _ = writeln!(s, "## Terms to decide first\n");
    let _ = writeln!(
        s,
        "From `docs/glossary/{}.md`, the \"Terms\" table: the terms the interface uses and the reviewers decide. A decision here applies everywhere the term is used.\n",
        lang.tag
    );
    let _ = writeln!(
        s,
        "| term | the choice | why, and the alternatives | your decision |\n|---|---|---|---|"
    );
    for t in hard {
        let _ = writeln!(
            s,
            "| {} | {} | {} |  |",
            t.english.replace('|', "\\|"),
            t.target.replace('|', "\\|"),
            t.why.replace('|', "\\|")
        );
    }
    let _ = writeln!(s, "\n## What happens next\n\n{}\n", next_steps(lang));
    let mut current: Option<(Group, String)> = None;
    for g in Group::ALL {
        let mut page_no = 0usize;
        let in_group: Vec<&Row> = rows.iter().filter(|r| r.group == g).collect();
        let _ = writeln!(
            s,
            "\n---\n\n## Group {}. {} — {} rows\n",
            g.letter(),
            g.title(),
            in_group.len()
        );
        if in_group.is_empty() {
            let _ = writeln!(s, "(no rows)\n");
            continue;
        }
        let _ = writeln!(s, "{}\n", g.covers());
        for r in in_group {
            if current
                .as_ref()
                .is_none_or(|(cg, cp)| *cg != g || *cp != r.page)
            {
                page_no += 1;
                let n = rows
                    .iter()
                    .filter(|x| x.group == g && x.page == r.page)
                    .count();
                let _ = writeln!(s, "\n### {}.{page_no} {} — {n} rows\n", g.letter(), r.page);
                let _ = writeln!(
                    s,
                    "{}",
                    column_header(signed, lang.endonym, lang.is_draft())
                );
                current = Some((g, r.page.clone()));
            }
            let ph = if r.placeholders.is_empty() {
                "—".to_owned()
            } else {
                r.placeholders
                    .iter()
                    .map(|p| format!("{{{p}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            if signed {
                let _ = writeln!(
                    s,
                    "| `{}` | {} | {} | {} | {} | {} | {} |  |",
                    r.key,
                    escape_cell(&r.situation),
                    escape_cell(&r.english),
                    escape_cell(&r.target),
                    escape_cell(&ph),
                    escape_cell(&r.note),
                    r.state.map_or("", ReviewState::label),
                );
            } else {
                let _ = writeln!(
                    s,
                    "| `{}` | {} | {} | {} | {} | {} |  |",
                    r.key,
                    escape_cell(&r.situation),
                    escape_cell(&r.english),
                    escape_cell(&r.target),
                    escape_cell(&ph),
                    escape_cell(&r.note)
                );
            }
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------------- reading a sheet back

/// A row as the sheet shows it (after unescaping).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetRow {
    pub group: Group,
    pub key: String,
    pub english: String,
    pub target: String,
    /// The "since sign-off" cell, when the sheet has that column.
    pub state: Option<String>,
}

/// Split a table line into its cells at the bars that are not escaped.
fn split_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line
        .strip_suffix('|')
        .filter(|_| !line.ends_with("\\|"))
        .unwrap_or(line);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cur.push('\\');
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '|' => cells.push(std::mem::take(&mut cur)),
            other => cur.push(other),
        }
    }
    cells.push(cur);
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

/// The language a sheet says it is for (`Language `ja``), if its header says.
pub fn sheet_language(sheet: &str) -> Option<String> {
    let at = sheet.find("**Language `")? + "**Language `".len();
    let end = sheet[at..].find('`')?;
    Some(sheet[at..at + end].to_owned())
}

/// The message rows of a sheet: the lines that begin with a key in backticks, each under its group heading.
pub fn parse_sheet(sheet: &str) -> Vec<SheetRow> {
    let mut group = None;
    let mut with_state = false;
    let mut rows = Vec::new();
    for line in sheet.lines() {
        if let Some(rest) = line.strip_prefix("## Group ") {
            group = rest
                .chars()
                .next()
                .and_then(|c| Group::from_letter(&c.to_string()));
            continue;
        }
        if line.starts_with("| key |") {
            with_state = line.contains("| since sign-off |");
            continue;
        }
        let Some(g) = group else { continue };
        if !line.starts_with("| `") {
            continue;
        }
        let cells = split_cells(line);
        if cells.len() < 4 {
            continue;
        }
        rows.push(SheetRow {
            group: g,
            key: cells[0].trim_matches('`').to_owned(),
            english: unescape_cell(&cells[2]),
            target: unescape_cell(&cells[3]),
            state: with_state.then(|| cells.get(6).cloned().unwrap_or_default()),
        });
    }
    rows
}

/// One way a sheet differs from the catalogs.
#[derive(Debug, PartialEq, Eq)]
pub enum Drift {
    /// The English text of a row is not the catalog's.
    EnglishDiffers { key: String },
    /// The target-language text of a row is not the catalog's.
    TargetDiffers { key: String },
    /// A catalog message that the sheet does not have.
    MissingFromSheet { key: String },
    /// A key the sheet names that the catalogs (the target ones) do not have.
    NotInCatalog { key: String },
    /// A key twice in the sheet.
    Duplicate { key: String },
    /// A row in another group than the one the catalogs' context puts it in.
    WrongGroup {
        key: String,
        found: Group,
        expected: Group,
    },
    /// The "since sign-off" cell is not what the baseline says now.
    StateDiffers { key: String },
}

/// Compare a sheet with the expected rows.
pub fn check_against(sheet: &str, rows: &[Row]) -> Vec<Drift> {
    let seen = parse_sheet(sheet);
    let mut drift = Vec::new();
    let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &seen {
        *counted.entry(&r.key).or_default() += 1;
    }
    for (key, n) in &counted {
        if *n > 1 {
            drift.push(Drift::Duplicate {
                key: (*key).to_owned(),
            });
        }
    }
    for r in &seen {
        match rows
            .iter()
            .find(|e| e.key == r.key && (e.group == Group::C) == (r.group == Group::C))
        {
            None => drift.push(Drift::NotInCatalog { key: r.key.clone() }),
            Some(e) => {
                if e.english != r.english {
                    drift.push(Drift::EnglishDiffers { key: r.key.clone() });
                }
                if e.target != r.target {
                    drift.push(Drift::TargetDiffers { key: r.key.clone() });
                }
                if e.group != r.group {
                    drift.push(Drift::WrongGroup {
                        key: r.key.clone(),
                        found: r.group,
                        expected: e.group,
                    });
                }
                if let (Some(want), Some(got)) = (e.state, &r.state)
                    && want.label() != got
                {
                    drift.push(Drift::StateDiffers { key: r.key.clone() });
                }
            }
        }
    }
    for e in rows {
        if !seen
            .iter()
            .any(|r| r.key == e.key && (r.group == Group::C) == (e.group == Group::C))
        {
            drift.push(Drift::MissingFromSheet { key: e.key.clone() });
        }
    }
    drift
}

/// Compare a sheet with the live catalogs of the language the sheet is for (or `fallback`).
pub fn check(sheet: &str, fallback: Option<&str>) -> Result<(String, Vec<Drift>, Vec<Row>), Error> {
    let tag = sheet_language(sheet)
        .or_else(|| fallback.map(str::to_owned))
        .ok_or_else(|| {
            Error::UnknownLanguage("(the sheet does not say its language; pass --lang)".to_owned())
        })?;
    let rows = live_rows(&tag)?;
    let drift = check_against(sheet, &rows);
    Ok((tag, drift, rows))
}

// ---------------------------------------------------------------------------------------------------- baselines and status

/// The baseline file text for one program's catalog of a language as it stands now.
pub fn baseline_for(tag: &str, program: Program) -> Result<String, Error> {
    let lang = planeter_i18n::languages::by_tag(tag)
        .ok_or_else(|| Error::UnknownLanguage(tag.to_owned()))?;
    let entries = match program {
        Program::Web => planeter_web::catalog::CATALOGS
            .iter()
            .find(|d| std::ptr::eq(d.lang, lang))
            .map(|d| d.entries),
        Program::Client => planeter_client::catalog::CATALOGS
            .iter()
            .find(|d| std::ptr::eq(d.lang, lang))
            .map(|d| d.entries),
    }
    .ok_or_else(|| Error::UnknownLanguage(tag.to_owned()))?;
    Ok(baseline::render(entries, tag))
}

/// One line per signed-off language for the release notes: who signed, when, and how many messages are newer than that (per program).
pub fn status_lines() -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for lang in planeter_i18n::languages::LANGUAGES
        .iter()
        .filter(|l| l.sign_off().is_some())
    {
        let src = live_sources(lang.tag)?;
        let (wu, wc, wn) = baseline::counts(
            src.web_target,
            src.web_baseline.as_ref().expect("signed off"),
        );
        let (cu, cc, cn) = baseline::counts(
            src.client_target,
            src.client_baseline.as_ref().expect("signed off"),
        );
        let sign = lang.sign_off().expect("signed off");
        out.push(format!(
            "{} ({}): signed off by {} on {}; {} message(s) newer than the sign-off (server {} changed + {} new, client {} changed + {} new); {} unchanged.",
            lang.tag,
            lang.endonym,
            sign.reviewers.join(", "),
            sign.date,
            wc + wn + cc + cn,
            wc,
            wn,
            cc,
            cn,
            wu + cu
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------- the date

/// `YYYY-MM-DD` (UTC) of a number of days since 1970-01-01 (Howard Hinnant's civil-from-days).
pub fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
