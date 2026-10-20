//! RFC 012 D-2, D-3(c), D-5 (presentation): checks over the one stylesheet, `static/app.css`, as tests.
//!
//! The stylesheet is small and ours, so it is read with a few lines of hand-written CSS reading (no CSS crate):
//! rules, `@media` blocks, custom properties. Every threshold below is written here, independently of the
//! stylesheet, and every check is shown able to fail on a planted stylesheet.
//!
//! What is checked: the contrast of every foreground/background pair the rules use, in both themes (text 4.5:1;
//! the borders that are the only identification of a control, and the focus ring, 3:1); logical properties only;
//! an explicit visible focus style that is never removed; no motion without a reduced-motion guard; a
//! 24 CSS px minimum size on every standalone target; forced colours and `color-scheme` present.

use std::collections::BTreeMap;

const CSS: &str = include_str!("../static/app.css");

// ---------------------------------------------------------------------------------------------------------
// A small reader

#[derive(Debug, Clone)]
struct Rule {
    selectors: Vec<String>,
    decls: Vec<(String, String)>,
    /// The `@media` conditions this rule sits in, outermost first, lower-cased, spaces removed.
    media: Vec<String>,
}

impl Rule {
    fn get(&self, prop: &str) -> Option<&str> {
        self.decls
            .iter()
            .rev()
            .find(|(p, _)| p == prop)
            .map(|(_, v)| v.as_str())
    }
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i..].find("*/") {
            Some(j) => rest = &rest[i + j + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_block(css: &str, media: &[String], out: &mut Vec<Rule>) {
    let mut rest = css.trim();
    while !rest.is_empty() {
        let Some(open) = rest.find('{') else { break };
        let prelude = rest[..open].trim();
        let mut depth = 0;
        let mut close = None;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.expect("an unclosed block in the stylesheet");
        let body = &rest[open + 1..close];
        if let Some(cond) = prelude.strip_prefix("@media") {
            let mut m = media.to_vec();
            m.push(
                cond.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_lowercase(),
            );
            parse_block(body, &m, out);
        } else {
            let decls = body
                .split(';')
                .filter_map(|d| {
                    let (p, v) = d.split_once(':')?;
                    Some((p.trim().to_lowercase(), v.trim().to_owned()))
                })
                .collect();
            out.push(Rule {
                selectors: prelude.split(',').map(|s| s.trim().to_owned()).collect(),
                decls,
                media: media.to_vec(),
            });
        }
        rest = rest[close + 1..].trim();
    }
}

fn parse(css: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    parse_block(&strip_comments(css), &[], &mut out);
    out
}

// ---------------------------------------------------------------------------------------------------------
// Colours

#[derive(Clone, Copy, PartialEq, Debug)]
enum Theme {
    Light,
    Dark,
}

/// The custom properties of a theme: `:root` outside any media block, over which the dark block is laid.
fn tokens(rules: &[Rule], theme: Theme) -> BTreeMap<String, String> {
    let mut t = BTreeMap::new();
    for r in rules {
        let root = r.selectors.iter().any(|s| s == ":root");
        let plain = r.media.is_empty();
        let dark = r.media == ["(prefers-color-scheme:dark)"];
        if root && (plain || (dark && theme == Theme::Dark)) {
            for (p, v) in &r.decls {
                if p.starts_with("--") {
                    t.insert(p.clone(), v.clone());
                }
            }
        }
    }
    t
}

fn resolve(value: &str, t: &BTreeMap<String, String>) -> Result<(u8, u8, u8), String> {
    let v = value.trim();
    if let Some(name) = v.strip_prefix("var(").and_then(|x| x.strip_suffix(')')) {
        let name = name.trim();
        let inner = t
            .get(name)
            .ok_or_else(|| format!("{name} is not defined"))?;
        return resolve(inner, t);
    }
    let hex = v
        .strip_prefix('#')
        .ok_or_else(|| format!("cannot read the colour {v:?}"))?;
    let h = |s: &str| u8::from_str_radix(s, 16).map_err(|_| format!("bad hex {v:?}"));
    match hex.len() {
        3 => {
            let d = |i: usize| h(&hex[i..=i].repeat(2));
            Ok((d(0)?, d(1)?, d(2)?))
        }
        6 => Ok((h(&hex[0..2])?, h(&hex[2..4])?, h(&hex[4..6])?)),
        _ => Err(format!("cannot read the colour {v:?}")),
    }
}

fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let f = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

fn ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The colour a declaration value names: a `background`/`border` shorthand holds it among other words.
fn colour_word(value: &str) -> Option<&str> {
    value
        .split_whitespace()
        .find(|w| w.starts_with("var(--") || w.starts_with('#'))
}

/// Selectors whose borders are the only visual identification of a control (WCAG 1.4.11): they need 3:1.
const CONTROL_BORDERS: &[&str] = &["input", "button", ".skip-link"];
const TEXT_MIN: f64 = 4.5;
const NON_TEXT_MIN: f64 = 3.0;

/// One measured pair: what it is, where it is used, its ratio and the threshold it must meet.
#[derive(Debug, Clone)]
struct Row {
    theme: Theme,
    what: &'static str,
    selector: String,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    ratio: f64,
    min: f64,
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Every pair the stylesheet uses, in both themes, and what could not be read.
fn contrast_rows(css: &str) -> (Vec<Row>, Vec<String>) {
    let rules = parse(css);
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for theme in [Theme::Light, Theme::Dark] {
        let t = tokens(&rules, theme);
        // The page's colours come from `body`.
        let body = rules
            .iter()
            .find(|r| r.media.is_empty() && r.selectors.iter().any(|s| s == "body"));
        let page = |prop: &str| -> String {
            body.and_then(|r| {
                r.get(prop).or_else(|| {
                    (prop == "background-color")
                        .then(|| r.get("background"))
                        .flatten()
                })
            })
            .unwrap_or("")
            .to_owned()
        };
        let (body_fg, body_bg) = (page("color"), page("background-color"));
        let (Ok(fg0), Ok(bg0)) = (resolve(&body_fg, &t), resolve(&body_bg, &t)) else {
            problems.push(format!("{theme:?}: the body's colour or background cannot be read ({body_fg:?}, {body_bg:?})"));
            continue;
        };
        rows.push(Row {
            theme,
            what: "text",
            selector: "body".into(),
            fg: fg0,
            bg: bg0,
            ratio: ratio(fg0, bg0),
            min: TEXT_MIN,
        });
        for r in &rules {
            // Forced colours replace the author's colours with the system's: nothing to compute.
            if r.media.iter().any(|m| m.contains("forced-colors")) {
                continue;
            }
            let sel = r.selectors.join(", ");
            let fg_decl = r.get("color");
            let bg_decl = r.get("background").or_else(|| r.get("background-color"));
            if fg_decl.is_some() || bg_decl.is_some() {
                let bg = match bg_decl.and_then(colour_word) {
                    Some(w) => resolve(w, &t),
                    None => Ok(bg0),
                };
                let fg = match fg_decl {
                    Some(v) => resolve(v, &t),
                    None => Ok(fg0),
                };
                match (fg, bg) {
                    (Ok(fg), Ok(bg)) => rows.push(Row {
                        theme,
                        what: "text",
                        selector: sel.clone(),
                        fg,
                        bg,
                        ratio: ratio(fg, bg),
                        min: TEXT_MIN,
                    }),
                    (Err(e), _) | (_, Err(e)) => problems.push(format!("{theme:?}: `{sel}`: {e}")),
                }
            }
            // Borders and outlines of controls and of the focus ring.
            for (prop, what) in [
                ("border", "control border"),
                ("border-color", "control border"),
                ("outline", "focus ring"),
                ("outline-color", "focus ring"),
            ] {
                let Some(v) = r.get(prop) else { continue };
                let Some(w) = colour_word(v) else { continue };
                let control = r
                    .selectors
                    .iter()
                    .any(|s| CONTROL_BORDERS.contains(&s.as_str()) || s.contains(":focus-visible"));
                if !control {
                    continue;
                }
                let bg = match r.get("background").and_then(colour_word) {
                    Some(b) => resolve(b, &t).unwrap_or(bg0),
                    None => bg0,
                };
                match resolve(w, &t) {
                    Ok(c) => rows.push(Row {
                        theme,
                        what,
                        selector: sel.clone(),
                        fg: c,
                        bg,
                        ratio: ratio(c, bg),
                        min: NON_TEXT_MIN,
                    }),
                    Err(e) => problems.push(format!("{theme:?}: `{sel}`: {e}")),
                }
            }
        }
        // The controls must say their border colour: a browser-default border cannot be computed here.
        for need in CONTROL_BORDERS.iter().filter(|s| **s != ".skip-link") {
            let has = rules.iter().any(|r| {
                r.media.is_empty()
                    && r.selectors.iter().any(|s| s == need)
                    && (r.get("border").and_then(colour_word).is_some()
                        || r.get("border-color").is_some())
            });
            if !has {
                problems.push(format!("{theme:?}: `{need}` has no explicit border colour (the browser's default cannot be checked)"));
            }
        }
    }
    (rows, problems)
}

/// Findings of the contrast test over a stylesheet, both themes.
fn contrast_findings(css: &str) -> Vec<String> {
    let (rows, mut out) = contrast_rows(css);
    for r in rows.iter().filter(|r| r.ratio < r.min) {
        out.push(format!(
            "{:?}: the {} of `{}` is {:.2}:1 (< {})",
            r.theme, r.what, r.selector, r.ratio, r.min
        ));
    }
    out.sort();
    out.dedup();
    out
}

/// `STYLESHEET_REPORT=<file>` writes the table of every pair (the review request's contrast table).
#[test]
fn the_contrast_table_can_be_written() {
    let (rows, problems) = contrast_rows(CSS);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(rows.len() > 20, "only {} pairs were found", rows.len());
    if let Ok(path) = std::env::var("STYLESHEET_REPORT") {
        let mut t = String::from(
            "| theme | pair | where used | foreground | background | ratio | threshold |\n|---|---|---|---|---|---|---|\n",
        );
        for r in &rows {
            t.push_str(&format!(
                "| {:?} | {} | `{}` | {} | {} | {:.2}:1 | {}:1 |\n",
                r.theme,
                r.what,
                r.selector,
                hex(r.fg),
                hex(r.bg),
                r.ratio,
                r.min
            ));
        }
        std::fs::write(path, t).unwrap();
    }
}

// ---------------------------------------------------------------------------------------------------------
// Direction, focus, motion, shortcuts, target size, forced colours

fn direction_findings(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    for r in parse(css) {
        for (p, v) in &r.decls {
            let physical_prop = matches!(p.as_str(), "left" | "right" | "clear" | "float")
                || p.starts_with("margin-left")
                || p.starts_with("margin-right")
                || p.starts_with("padding-left")
                || p.starts_with("padding-right")
                || p.starts_with("border-left")
                || p.starts_with("border-right")
                || (p.starts_with("border-") && (p.contains("-left-") || p.contains("-right-")));
            let physical_value = matches!(p.as_str(), "text-align" | "float" | "clear")
                && matches!(v.as_str(), "left" | "right");
            if (physical_prop && !(matches!(p.as_str(), "float" | "clear") && !physical_value))
                || physical_value
            {
                out.push(format!("`{}`: {p}: {v}", r.selectors.join(", ")));
            }
        }
    }
    out
}

fn is_removal(p: &str, v: &str) -> bool {
    (p == "outline" && matches!(v.split_whitespace().next(), Some("none" | "0" | "0px")))
        || (p == "outline-style" && v == "none")
        || (p == "outline-width" && matches!(v, "0" | "0px"))
}

fn px(v: &str) -> Option<f64> {
    let v = v.trim();
    if let Some(n) = v.strip_suffix("px") {
        return n.trim().parse().ok();
    }
    if let Some(n) = v.strip_suffix("rem") {
        return n.trim().parse::<f64>().ok().map(|x| x * 16.0);
    }
    None
}

fn focus_findings(css: &str) -> Vec<String> {
    let rules = parse(css);
    let mut out = Vec::new();
    let focus: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.selectors.iter().any(|s| s.contains(":focus-visible")))
        // The rules that draw the ring (another `:focus-visible` rule may style something else, e.g. an underline).
        .filter(|r| r.decls.iter().any(|(p, _)| p.starts_with("outline")))
        // The forced-colours block restates the ring in system colours; the ring itself is the rule outside it.
        .filter(|r| !r.media.iter().any(|m| m.contains("forced-colors")))
        .collect();
    for r in &rules {
        for (p, v) in &r.decls {
            if is_removal(p, v) && !r.selectors.iter().all(|s| s.contains(":focus-visible")) {
                out.push(format!(
                    "`{}` removes the outline ({p}: {v})",
                    r.selectors.join(", ")
                ));
            }
            if is_removal(p, v) && r.selectors.iter().all(|s| s.contains(":focus-visible")) {
                out.push(format!(
                    "`{}` removes the focus outline itself",
                    r.selectors.join(", ")
                ));
            }
        }
    }
    if focus.is_empty() {
        out.push("there is no :focus-visible rule".into());
    }
    // One explicit style, applied to every kind of target: a bare `:focus-visible` (or the four named).
    let covers = |name: &str| {
        focus.iter().any(|r| {
            r.selectors
                .iter()
                .any(|s| s == ":focus-visible" || s == &format!("{name}:focus-visible"))
        })
    };
    for name in ["a", "button", "input", ".skip-link"] {
        if !covers(name) {
            out.push(format!("no :focus-visible style covers `{name}`"));
        }
    }
    let mut width_ok = false;
    for r in &focus {
        let o = r.get("outline");
        let width = r
            .get("outline-width")
            .and_then(px)
            .or_else(|| o.and_then(|v| v.split_whitespace().find_map(px)));
        let solid = r.get("outline-style") == Some("solid")
            || o.is_some_and(|v| v.split_whitespace().any(|w| w == "solid"));
        if width.is_some_and(|w| w >= 2.0) && solid {
            width_ok = true;
        }
        if r.get("outline-offset").is_none() {
            out.push(format!(
                "`{}` has no outline-offset (the ring should stand off the control)",
                r.selectors.join(", ")
            ));
        }
    }
    if !focus.is_empty() && !width_ok {
        out.push("no :focus-visible rule draws a solid outline of at least 2px".into());
    }
    out
}

fn motion_findings(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    for r in parse(css) {
        for (p, v) in &r.decls {
            let moving = p.starts_with("animation")
                || p.starts_with("transition")
                || (p == "scroll-behavior" && v != "auto");
            let guarded = r
                .media
                .iter()
                .any(|m| m == "(prefers-reduced-motion:no-preference)");
            if moving && !guarded {
                out.push(format!(
                    "`{}`: {p}: {v} without a prefers-reduced-motion guard",
                    r.selectors.join(", ")
                ));
            }
        }
    }
    out
}

fn shortcut_findings(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    for r in parse(css) {
        for (p, v) in &r.decls {
            if v.contains("!important") {
                out.push(format!("`{}`: {p} uses !important", r.selectors.join(", ")));
            }
            if (p == "overflow" || p == "overflow-x")
                && v == "hidden"
                && !r.selectors.iter().any(|s| s == ".visually-hidden")
            {
                out.push(format!(
                    "`{}`: {p}: hidden hides overflow instead of fixing it",
                    r.selectors.join(", ")
                ));
            }
        }
    }
    out
}

/// Standalone targets: the selectors that style them. Links in running text are exempt and are not listed.
const TARGETS: &[&str] = &[
    "header a",
    "nav.repo a",
    "th a",
    "td a",
    ".crumbs a",
    "main > p > a",
    ".skip-link",
    "input",
    "button",
];
const TARGET_MIN: f64 = 24.0;

fn target_findings(css: &str) -> Vec<String> {
    let rules = parse(css);
    let mut out = Vec::new();
    for want in TARGETS {
        let hits: Vec<&Rule> = rules
            .iter()
            .filter(|r| r.media.is_empty() && r.selectors.iter().any(|s| s == want))
            .collect();
        let max = |prop: &str| {
            hits.iter()
                .filter_map(|r| r.get(prop).and_then(px))
                .fold(0.0_f64, f64::max)
        };
        if max("min-height") < TARGET_MIN {
            out.push(format!("`{want}` has no min-height of {TARGET_MIN}px"));
        }
        if max("min-width") < TARGET_MIN {
            out.push(format!("`{want}` has no min-width of {TARGET_MIN}px"));
        }
        // A link is `inline` by default, and min-height does nothing to an inline box.
        let is_link = want.ends_with(" a") || want.ends_with("> a") || *want == ".skip-link";
        if is_link && *want != ".skip-link" {
            let display = hits
                .iter()
                .filter_map(|r| r.get("display"))
                .next_back()
                .unwrap_or("inline");
            if !matches!(
                display,
                "inline-block" | "inline-flex" | "block" | "flex" | "grid" | "inline-grid"
            ) {
                out.push(format!(
                    "`{want}` is `display: {display}`: min-height has no effect on it"
                ));
            }
        }
    }
    out
}

fn scheme_findings(css: &str) -> Vec<String> {
    let rules = parse(css);
    let mut out = Vec::new();
    let root_scheme = rules
        .iter()
        .filter(|r| r.media.is_empty() && r.selectors.iter().any(|s| s == ":root"))
        .any(|r| {
            r.get("color-scheme")
                .is_some_and(|v| v.split_whitespace().collect::<Vec<_>>() == ["light", "dark"])
        });
    if !root_scheme {
        out.push("`:root` does not declare `color-scheme: light dark` (form controls stay light in the dark theme)".into());
    }
    let forced: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.media.iter().any(|m| m == "(forced-colors:active)"))
        .collect();
    if forced.is_empty() {
        out.push("there is no `@media (forced-colors: active)` block".into());
    }
    let has = |sel: &str, prop: &str| {
        forced
            .iter()
            .any(|r| r.selectors.iter().any(|s| s.contains(sel)) && r.get(prop).is_some())
    };
    if !has(".badge", "border") && !has(".badge", "border-color") {
        out.push("forced colours: `.badge` keeps no border".into());
    }
    if !has(":focus-visible", "outline") && !has(":focus-visible", "outline-color") {
        out.push("forced colours: the focus ring is not restated".into());
    }
    if !has("button", "border") && !has("button", "border-color") {
        out.push("forced colours: `button` keeps no border".into());
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// The tests on the real stylesheet

#[test]
fn every_colour_pair_the_stylesheet_uses_has_enough_contrast_in_both_themes() {
    let f = contrast_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn the_stylesheet_uses_logical_properties_only() {
    let f = direction_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn focus_has_one_explicit_visible_style_and_is_never_removed() {
    let f = focus_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn nothing_moves_without_a_reduced_motion_guard() {
    let f = motion_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn no_shortcut_hides_overflow_or_wins_a_specificity_fight() {
    let f = shortcut_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn every_standalone_target_is_at_least_24_css_px() {
    let f = target_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn the_page_declares_both_colour_schemes_and_restates_itself_for_forced_colours() {
    let f = scheme_findings(CSS);
    assert!(f.is_empty(), "{f:#?}");
}

// ---------------------------------------------------------------------------------------------------------
// Each check can fail

/// A stylesheet that passes every check; each planted failure below changes one thing in it.
const GOOD: &str = r#"
:root{color-scheme:light dark;--bg:#fff;--fg:#1a1a1a;--muted:#666;--control:#767676;--accent:#2a5db0;--line:#ddd}
@media (prefers-color-scheme:dark){:root{--bg:#111;--fg:#eee;--muted:#aaa;--control:#8c8c8c;--accent:#7aa7ff}}
body{margin:0;color:var(--fg);background:var(--bg)}
header a,nav.repo a,th a,td a,.crumbs a,main > p > a,.skip-link{display:inline-block;min-height:24px;min-width:24px}
input,button{min-height:24px;min-width:24px;border:1px solid var(--control);background:var(--bg);color:var(--fg)}
.skip-link{border:1px solid var(--control);background:var(--bg)}
.muted{color:var(--muted)}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
header .who{margin-inline-start:auto}
td,th{text-align:start}
@media (prefers-reduced-motion:no-preference){a{transition:color .1s}}
@media (forced-colors:active){.badge{border:1px solid CanvasText}:focus-visible{outline:2px solid Highlight}button{border:1px solid ButtonBorder}}
"#;

fn replaced(from: &str, to: &str) -> String {
    assert!(GOOD.contains(from), "the plant does not apply: {from:?}");
    GOOD.replacen(from, to, 1)
}

#[test]
fn the_good_stylesheet_passes_every_check() {
    assert_eq!(contrast_findings(GOOD), Vec::<String>::new());
    assert_eq!(direction_findings(GOOD), Vec::<String>::new());
    assert_eq!(focus_findings(GOOD), Vec::<String>::new());
    assert_eq!(motion_findings(GOOD), Vec::<String>::new());
    assert_eq!(shortcut_findings(GOOD), Vec::<String>::new());
    assert_eq!(target_findings(GOOD), Vec::<String>::new());
    assert_eq!(scheme_findings(GOOD), Vec::<String>::new());
}

#[test]
fn contrast_fails_on_a_weak_text_pair_in_either_theme_and_on_a_weak_control_border_or_ring() {
    // light text pair
    assert!(!contrast_findings(&replaced("--muted:#666", "--muted:#999")).is_empty());
    // dark text pair only
    let f = contrast_findings(&replaced("--muted:#aaa", "--muted:#444"));
    assert!(
        f.iter().all(|x| x.starts_with("Dark")) && !f.is_empty(),
        "{f:?}"
    );
    // a control's border in the dark theme only
    let f = contrast_findings(&replaced("--control:#8c8c8c", "--control:#222"));
    assert!(
        f.iter()
            .any(|x| x.starts_with("Dark") && x.contains("input")),
        "{f:?}"
    );
    // the focus ring
    let f = contrast_findings(&replaced(
        "outline:2px solid var(--accent)",
        "outline:2px solid var(--line)",
    ));
    assert!(f.iter().any(|x| x.contains(":focus-visible")), "{f:?}");
    // a control with no border colour of its own
    let f = contrast_findings(&replaced(
        "input,button{min-height:24px;min-width:24px;border:1px solid var(--control);",
        "input,button{min-height:24px;min-width:24px;",
    ));
    assert!(
        f.iter().any(|x| x.contains("no explicit border colour")),
        "{f:?}"
    );
    // an undefined token is a finding, not a pass
    assert!(!contrast_findings(&replaced("color:var(--muted)", "color:var(--nope)")).is_empty());
    // decorative lines may be faint: `--line` on a rule that is not a control is not a finding
    assert!(
        contrast_findings(&replaced(
            ".muted{",
            "th{border-bottom:1px solid var(--line)}.muted{"
        ))
        .is_empty()
    );
}

#[test]
fn direction_fails_on_every_physical_declaration() {
    for (from, to) in [
        ("margin-inline-start:auto", "margin-left:auto"),
        ("margin-inline-start:auto", "margin-right:1rem"),
        ("margin-inline-start:auto", "padding-left:1rem"),
        ("margin-inline-start:auto", "padding-right:1rem"),
        ("margin-inline-start:auto", "border-left:1px solid #000"),
        ("margin-inline-start:auto", "border-right-color:#000"),
        ("margin-inline-start:auto", "left:0"),
        ("margin-inline-start:auto", "right:0"),
        ("margin-inline-start:auto", "float:left"),
        ("text-align:start", "text-align:left"),
        ("text-align:start", "text-align:right"),
    ] {
        assert!(!direction_findings(&replaced(from, to)).is_empty(), "{to}");
    }
    assert!(
        direction_findings(&replaced(
            "margin-inline-start:auto",
            "inset-inline-start:0;margin-inline-end:1rem;float:inline-start"
        ))
        .is_empty()
    );
}

#[test]
fn focus_fails_when_the_outline_is_removed_or_the_ring_is_thin_or_missing() {
    assert!(!focus_findings(&replaced("input,button{", "a{outline:none}input,button{")).is_empty());
    assert!(!focus_findings(&replaced("input,button{", "a{outline:0}input,button{")).is_empty());
    assert!(
        !focus_findings(&replaced(
            "input,button{",
            "a{outline-style:none}input,button{"
        ))
        .is_empty()
    );
    assert!(
        !focus_findings(&replaced(
            ":focus-visible{outline:2px solid var(--accent);outline-offset:2px}",
            ""
        ))
        .is_empty()
    );
    assert!(
        !focus_findings(&replaced(
            "outline:2px solid var(--accent)",
            "outline:1px solid var(--accent)"
        ))
        .is_empty()
    );
    assert!(
        !focus_findings(&replaced(
            "outline:2px solid var(--accent);outline-offset:2px",
            "outline:2px solid var(--accent)"
        ))
        .is_empty()
    );
    assert!(
        !focus_findings(&replaced(
            ":focus-visible{outline:2px",
            "a:focus-visible{outline:2px"
        ))
        .is_empty(),
        "a style for links only leaves the rest"
    );
}

#[test]
fn motion_fails_without_the_guard() {
    assert!(
        !motion_findings(&replaced(
            "@media (prefers-reduced-motion:no-preference){a{transition:color .1s}}",
            "a{transition:color .1s}"
        ))
        .is_empty()
    );
    assert!(
        !motion_findings(&replaced(
            "input,button{",
            "html{scroll-behavior:smooth}input,button{"
        ))
        .is_empty()
    );
    assert!(
        !motion_findings(&replaced(
            "input,button{",
            "a{animation:spin 1s infinite}input,button{"
        ))
        .is_empty()
    );
}

#[test]
fn the_shortcuts_are_found() {
    assert!(
        !shortcut_findings(&replaced(
            "td,th{text-align:start}",
            "td,th{text-align:start !important}"
        ))
        .is_empty()
    );
    assert!(!shortcut_findings(&replaced("td,th{", "main{overflow:hidden}td,th{")).is_empty());
    assert!(
        shortcut_findings(&replaced(
            "td,th{",
            ".visually-hidden{overflow:hidden}td,th{"
        ))
        .is_empty()
    );
}

#[test]
fn target_size_fails_on_a_missing_minimum_or_an_inline_link() {
    assert!(!target_findings(&replaced("header a,nav.repo a,th a,td a,.crumbs a,main > p > a,.skip-link{display:inline-block;min-height:24px;min-width:24px}", "header a,nav.repo a,th a,td a,.crumbs a,main > p > a,.skip-link{display:inline-block;min-width:24px}")).is_empty());
    assert!(
        !target_findings(&replaced(
            "min-height:24px;min-width:24px}\ninput",
            "min-height:20px;min-width:24px}\ninput"
        ))
        .is_empty()
    );
    assert!(
        !target_findings(&replaced(
            "display:inline-block;min-height:24px",
            "display:inline;min-height:24px"
        ))
        .is_empty()
    );
    assert!(
        !target_findings(&replaced(
            "input,button{min-height:24px;min-width:24px;",
            "input,button{min-width:24px;"
        ))
        .is_empty()
    );
    assert!(
        !target_findings(&replaced("th a,", "")).is_empty(),
        "a target selector that no rule styles"
    );
}

#[test]
fn colour_scheme_and_forced_colours_fail_when_absent() {
    assert!(!scheme_findings(&replaced("color-scheme:light dark;", "")).is_empty());
    assert!(
        !scheme_findings(&replaced("color-scheme:light dark;", "color-scheme:light;")).is_empty()
    );
    assert!(!scheme_findings(&replaced("@media (forced-colors:active){.badge{border:1px solid CanvasText}:focus-visible{outline:2px solid Highlight}button{border:1px solid ButtonBorder}}", "")).is_empty());
    assert!(!scheme_findings(&replaced(".badge{border:1px solid CanvasText}", "")).is_empty());
    assert!(!scheme_findings(&replaced("button{border:1px solid ButtonBorder}", "")).is_empty());
}

#[test]
fn the_reader_reads_nested_blocks_and_comments() {
    let rules = parse(
        "/* a { color: red } */ a,b{color:#fff;margin:0} @media (a: b) { @media (c) { x{y:z} } }",
    );
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].selectors, ["a", "b"]);
    assert_eq!(rules[0].get("color"), Some("#fff"));
    assert_eq!(rules[1].media, ["(a:b)", "(c)"]);
}
