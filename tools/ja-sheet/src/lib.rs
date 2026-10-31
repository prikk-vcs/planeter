#![forbid(unsafe_code)]
//! `planeter-ja-sheet` — the one Japanese review sheet (RFC 012 D-3f, handoff `japanese-consolidated-sheet-handoff-v1`).
//!
//! The sheet is **generated** from the live catalogs of both programs (the server's and `planeter-client`'s), never written by hand:
//! every row's English and Japanese cell is the catalog's text, so what is reviewed is exactly what is compiled. [`check`] reads a
//! sheet back and reports every way it can have drifted from the catalogs: a cell that differs, a message the sheet lacks, a key the
//! catalogs do not have, a row twice, a row in the wrong group.
//!
//! Rows are grouped by what a reader sees (the reviewer may stop after any group): **A** the words that carry trust, **B** the pages
//! a person reads (page by page), **C** the command-line program, **D** developer- and operator-facing texts (optional for the
//! sign-off). The group, the page and the "where it appears" of a message come from `data/contexts.tsv` (the longest matching
//! pattern; a message that matches none is an error), and the note for the reviewer from `data/notes.tsv`.
//!
//! A message that is English-only (`ENGLISH_ONLY_KEYS` of the server's catalog) has no Japanese entry and is not in the sheet.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub const CONTEXTS: &str = include_str!("../data/contexts.tsv");
pub const NOTES: &str = include_str!("../data/notes.tsv");

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
    /// The English cell and the Japanese cell, as text (before the table escaping).
    pub english: String,
    pub japanese: String,
    pub placeholders: Vec<String>,
    pub note: String,
}

/// A message of either catalog, reduced to what the sheet needs: its key and its text for the singular and the other form.
#[derive(Clone, Copy, Debug)]
pub struct Msg<'a> {
    pub key: &'a str,
    pub one: &'a str,
    pub other: &'a str,
}

/// Why a sheet or its inputs cannot be used.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A message that no line of `contexts.tsv` covers.
    NoContext { program: Program, key: String },
    /// `contexts.tsv` is malformed (a line, with the reason).
    BadContexts(String),
    /// A Japanese message with no English one.
    NoEnglish { key: String },
    /// A note for a key that no catalog has.
    NoteForUnknownKey(String),
    /// `notes.tsv` is malformed.
    BadNotes(String),
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
                write!(f, "the Japanese message `{key}` has no English message")
            }
            Error::NoteForUnknownKey(k) => write!(
                f,
                "data/notes.tsv has a note for `{k}`, which no catalog has"
            ),
            Error::BadNotes(m) => write!(f, "data/notes.tsv: {m}"),
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

/// The parsed `notes.tsv`.
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

/// Placeholder names of a text, in order of appearance, without repeats.
fn placeholders_of(text: &str, into: &mut Vec<String>) {
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        match rest[i..].find('}') {
            Some(j) => {
                let name = &rest[i + 1..i + j];
                if !into.iter().any(|n| n == name) {
                    into.push(name.to_owned());
                }
                rest = &rest[i + j + 1..];
            }
            None => break,
        }
    }
}

/// The text of a message as one cell: one text when the forms agree, otherwise both, labelled.
pub fn message_cell(m: &Msg<'_>) -> String {
    if m.one == m.other {
        m.other.to_owned()
    } else {
        format!("[one] {} [other] {}", m.one, m.other)
    }
}

/// The rows of one program: every Japanese message (in its catalog order) with the English one beside it. A key the Japanese catalog
/// does not hold (English-only) is not a row.
pub fn rows_of(
    program: Program,
    english: &[Msg<'_>],
    japanese: &[Msg<'_>],
    contexts: &Contexts,
    notes: &BTreeMap<String, String>,
) -> Result<Vec<Row>, Error> {
    let mut rows = Vec::new();
    for ja in japanese {
        let Some(en) = english.iter().find(|e| e.key == ja.key) else {
            return Err(Error::NoEnglish {
                key: ja.key.to_owned(),
            });
        };
        let ctx = contexts
            .find(program, ja.key)
            .ok_or_else(|| Error::NoContext {
                program,
                key: ja.key.to_owned(),
            })?;
        let mut names = Vec::new();
        for text in [en.one, en.other, ja.one, ja.other] {
            placeholders_of(text, &mut names);
        }
        rows.push(Row {
            program,
            key: ja.key.to_owned(),
            group: ctx.group,
            page: ctx.page.clone(),
            situation: ctx.situation.clone(),
            english: message_cell(en),
            japanese: message_cell(ja),
            placeholders: names,
            note: notes.get(ja.key).cloned().unwrap_or_default(),
        });
    }
    Ok(rows)
}

/// Every row of the live catalogs, in the sheet's order: by group, then by page (in the order of `contexts.tsv`), and inside a page in
/// catalog order (the server's messages first, then the client's).
pub fn live_rows() -> Result<Vec<Row>, Error> {
    use planeter_web::catalog::{EN_DEF, JA_DEF};
    let contexts = Contexts::parse(CONTEXTS)?;
    let notes = parse_notes(NOTES)?;
    let web_en: Vec<Msg<'_>> = EN_DEF
        .entries
        .iter()
        .map(|e| Msg {
            key: e.key,
            one: e.one,
            other: e.other,
        })
        .collect();
    let web_ja: Vec<Msg<'_>> = JA_DEF
        .entries
        .iter()
        .map(|e| Msg {
            key: e.key,
            one: e.one,
            other: e.other,
        })
        .collect();
    let cli = |lang: planeter_client::catalog::Lang| -> Vec<Msg<'static>> {
        lang.entries()
            .iter()
            .map(|e| Msg {
                key: e.key,
                one: e.text,
                other: e.text,
            })
            .collect()
    };
    let cli_en = cli(planeter_client::catalog::Lang::En);
    let cli_ja = cli(planeter_client::catalog::Lang::Ja);
    assemble(&web_en, &web_ja, &cli_en, &cli_ja, &contexts, &notes)
}

/// [`live_rows`] over given tables (so that the tests can plant catalogs).
pub fn assemble(
    web_en: &[Msg<'_>],
    web_ja: &[Msg<'_>],
    cli_en: &[Msg<'_>],
    cli_ja: &[Msg<'_>],
    contexts: &Contexts,
    notes: &BTreeMap<String, String>,
) -> Result<Vec<Row>, Error> {
    let mut rows = rows_of(Program::Web, web_en, web_ja, contexts, notes)?;
    rows.extend(rows_of(Program::Client, cli_en, cli_ja, contexts, notes)?);
    // A note must belong to a key of the sheet's catalogs (English-only keys keep theirs out: they are not in the sheet).
    let known: BTreeSet<&str> = web_en.iter().chain(cli_en).map(|m| m.key).collect();
    for key in notes.keys() {
        if !known.contains(key.as_str()) {
            return Err(Error::NoteForUnknownKey(key.clone()));
        }
    }
    rows.sort_by_key(|r| (r.group, contexts.page_rank(&r.page)));
    Ok(rows)
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

/// A term the reviewers decide first (a "Hard term" row of `docs/glossary.md` section 5.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardTerm {
    pub english: String,
    pub japanese: String,
    pub why: String,
    pub status: String,
}

/// The hard terms of the glossary's section 5.1 (the rows whose status begins with "Hard term").
pub fn hard_terms(glossary: &str) -> Vec<HardTerm> {
    let mut on = false;
    let mut header_done = false;
    let mut out = Vec::new();
    for line in glossary.lines() {
        if line.starts_with("### ") || line.starts_with("## ") {
            on = line.starts_with("### 5.1");
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
        if let [english, japanese, _shown, why, status] = cells[..]
            && status.starts_with("Hard term")
        {
            out.push(HardTerm {
                english: english.to_owned(),
                japanese: japanese.to_owned(),
                why: why.to_owned(),
                status: status.to_owned(),
            });
        }
    }
    out
}

/// FNV-1a over every row's key, English and Japanese: an identifier for "this wording", for the record of a sign-off. It changes when
/// any shown text changes; it is not a cryptographic hash.
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
        feed(&r.japanese);
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

const JA_INSTRUCTIONS: &str = "\
1. **これは何か。** planeter が画面とコマンドラインで表示する文章のすべてを、英語（確定済み）と日本語（草案）で並べた表です。表の内容はプログラムのメッセージカタログから自動で作られており、出荷される文章とまったく同じです。\n\
2. **やること。** 直したい行の最後の列「reviewer's correction」に、直した日本語の文を書いてください。問題がなければ「OK」と書くか、空欄のままにしてください（空欄は「このままでよい」という意味です）。他の列は編集しないでください。\n\
3. **途中でやめてよい。** グループ A、B、C、D の順に並んでいます。各グループの終わりでやめても構いません。A は信頼に関わる言葉なので、必ず最初に確認してください。D は開発者と運用者向けで、英語のままにすることもできます（その場合、D は表から消えます）。\n\
4. **{波括弧} の部分。** `{repo}` や `{count}` のような部分は、表示の瞬間にプログラムが値を入れます。綴りを変えず、日本語として自然な位置に入れてください。波括弧の中は翻訳しません。\n\
5. **最初に決めてほしい用語。** 下の表の用語は、決めた内容がすべての行に適用されます。先に決めてください。\n\
6. **このあと。** 修正はグループごとにカタログと用語集へ反映され（1 グループ 1 コミット）、確認の記録（誰が、いつ、どの表で）が残ります。そのあと日本語が公開されます。";

const EN_INSTRUCTIONS: &str = "\
1. **What this is.** Every sentence planeter shows, on its pages and on the command line, with the English (final) and the Japanese (a draft) side by side. The sheet is generated from the programs' message catalogs, so it is exactly what ships.\n\
2. **What to do.** Where a Japanese text should change, write the corrected Japanese in the last column, \"reviewer's correction\". If a row is fine, write \"OK\" or leave it empty (empty means: accept it as it is). Do not edit the other columns.\n\
3. **You may stop after any group.** The groups run A, B, C, D. A carries the words that tell a reader how far to trust what they see: read it first and in full. D is for developers and operators and may stay in English if the owner decides so (then D disappears from the sheet).\n\
4. **The {braces}.** Text like `{repo}` or `{count}` is filled in by the program when the sentence is shown. Keep it spelled exactly, in a place where it reads naturally in Japanese; do not translate what is inside the braces.\n\
5. **Terms to decide first.** The terms in the table below apply to every row that uses them; please decide them before the rows.\n\
6. **What happens next.** Corrections become catalog and glossary edits, one commit per group, and a record of the sign-off (who, when, which sheet) is made; then Japanese is offered to everyone.";

const NEXT_STEPS: &str = "\
1. Each group's corrections become edits of the Japanese catalog (`crates/planeter-web/src/catalog/ja.rs`, `crates/planeter-client/src/catalog/ja.rs`) and of the glossary's section 5 (`docs/glossary.md`) — **one commit per group**; a term decided in the table at the top is applied everywhere it is used, in the same commit that decides it.\n\
2. The sheet is regenerated after the last commit and checked (`planeter-ja-sheet --check`): every row says what the catalogs say.\n\
3. The sign-off is recorded: `JA_SIGNOFF` in the server's catalog (and `JA_REVIEWED` in both programs) are set together to **true** with the record — who reviewed, the date, the commit and the sheet's fingerprint (header above). A test refuses one without the other.\n\
4. Phase 2, still open after the sign-off: hide the lone English button of the language switcher when only English is offered (it is shown while Japanese is not), and finalize the Japanese wording of the accessibility statement (`docs/accessibility.md`).\n\
5. From then on the next feature that adds a Japanese message regenerates this sheet; nobody writes an addendum again.";

/// Render the whole sheet.
pub fn render(rows: &[Row], stamp: &Stamp, hard: &[HardTerm]) -> String {
    let mut s = String::new();
    let counts = counts(rows);
    let total: usize = counts.values().sum();
    let mark = if stamp.dirty {
        " (+ uncommitted changes)"
    } else {
        ""
    };
    let _ = writeln!(
        s,
        "# Japanese review sheet — planeter (one sheet, generated)\n"
    );
    let _ = writeln!(
        s,
        "**Generated from the catalogs of commit `{}`{mark} on {}.** Catalog fingerprint `{}`. {total} messages. \
Regenerate with `cargo run -p planeter-ja-sheet -- --out FILE`; check a sheet against the catalogs with `cargo run -p planeter-ja-sheet -- --check FILE`. \
Do not edit the English, Japanese or key columns by hand: they are the catalogs' text, and the check fails on any difference.\n",
        stamp.commit,
        stamp.date,
        fingerprint(rows)
    );
    let _ = writeln!(
        s,
        "**For:** the owner and a colleague, the two reviewers. Nothing is published until you have gone through it and the corrections are integrated.\n"
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
    let _ = writeln!(s, "\n## 何をするか（日本語）\n\n{JA_INSTRUCTIONS}\n");
    let _ = writeln!(s, "## What to do (English)\n\n{EN_INSTRUCTIONS}\n");
    let _ = writeln!(s, "## Terms to decide first\n");
    let _ = writeln!(
        s,
        "From `docs/glossary.md`, section 5.1: the terms the draft proposes and the reviewers decide. A decision here applies everywhere the term is used.\n"
    );
    let _ = writeln!(
        s,
        "| term | the draft's choice | why, and the alternatives | your decision |\n|---|---|---|---|"
    );
    for t in hard {
        let _ = writeln!(
            s,
            "| {} | {} | {} |  |",
            t.english.replace('|', "\\|"),
            t.japanese.replace('|', "\\|"),
            t.why.replace('|', "\\|")
        );
    }
    let _ = writeln!(s, "\n## What happens next\n\n{NEXT_STEPS}\n");
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
                    "| key | where it appears | English | Japanese draft | placeholders | note for the reviewer | reviewer's correction |\n|---|---|---|---|---|---|---|"
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
            let _ = writeln!(
                s,
                "| `{}` | {} | {} | {} | {} | {} |  |",
                r.key,
                escape_cell(&r.situation),
                escape_cell(&r.english),
                escape_cell(&r.japanese),
                escape_cell(&ph),
                escape_cell(&r.note)
            );
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
    pub japanese: String,
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

/// The message rows of a sheet: the lines that begin with a key in backticks, each under its group heading.
pub fn parse_sheet(sheet: &str) -> Vec<SheetRow> {
    let mut group = None;
    let mut rows = Vec::new();
    for line in sheet.lines() {
        if let Some(rest) = line.strip_prefix("## Group ") {
            group = rest
                .chars()
                .next()
                .and_then(|c| Group::from_letter(&c.to_string()));
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
        let key = cells[0].trim_matches('`').to_owned();
        rows.push(SheetRow {
            group: g,
            key,
            english: unescape_cell(&cells[2]),
            japanese: unescape_cell(&cells[3]),
        });
    }
    rows
}

/// One way a sheet differs from the catalogs.
#[derive(Debug, PartialEq, Eq)]
pub enum Drift {
    /// The English text of a row is not the catalog's.
    EnglishDiffers { key: String },
    /// The Japanese text of a row is not the catalog's.
    JapaneseDiffers { key: String },
    /// A catalog message that the sheet does not have.
    MissingFromSheet { key: String },
    /// A key the sheet names that the catalogs (the Japanese ones) do not have.
    NotInCatalog { key: String },
    /// A key twice in the sheet.
    Duplicate { key: String },
    /// A row in another group than the one the catalogs' context puts it in.
    WrongGroup {
        key: String,
        found: Group,
        expected: Group,
    },
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
                if e.japanese != r.japanese {
                    drift.push(Drift::JapaneseDiffers { key: r.key.clone() });
                }
                if e.group != r.group {
                    drift.push(Drift::WrongGroup {
                        key: r.key.clone(),
                        found: r.group,
                        expected: e.group,
                    });
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

/// Compare a sheet with the live catalogs.
pub fn check(sheet: &str) -> Result<Vec<Drift>, Error> {
    Ok(check_against(sheet, &live_rows()?))
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
