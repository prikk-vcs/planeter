//! Strict HTML/Markdown sanitization (RFC 003 T4 / `STD-6`) — the rendering-safety core.
//!
//! Repository content is **untrusted**: a README, a filename, an issue body may contain hostile HTML.
//! Anything that could become markup on a planeter page passes through here first. [`sanitize_html`]
//! runs `ammonia` (an html5ever-based sanitizer) which strips `<script>`/`<style>`, all `on*` event
//! handlers, and dangerous URL schemes (`javascript:`, script-bearing `data:`), keeping only a safe tag/
//! attribute allowlist. [`render_markdown`] renders CommonMark to HTML and then sanitizes it (Markdown
//! may embed raw HTML, so sanitizing the *output* is what makes it safe).
//!
//! Raw repository bytes (the raw file view, downloads) are never rendered as HTML at all — they are
//! served from the isolated content origin with download headers (see [`crate::security`]).

use pulldown_cmark::{Options, Parser, html};

/// Sanitize arbitrary HTML to a safe subset (scripts, event handlers, and `javascript:`/`data:` script
/// URLs removed). Safe to embed in a page.
pub fn sanitize_html(html: &str) -> String {
    ammonia::clean(html)
}

/// Render CommonMark to **sanitized** HTML. Tables/footnotes/strikethrough are enabled; the rendered
/// HTML (including any raw HTML the source embedded) is then run through [`sanitize_html`].
pub fn render_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(markdown, options);
    let mut raw = String::new();
    html::push_html(&mut raw, parser);
    demote_headings(&sanitize_html(&raw))
}

/// Move every heading in rendered content one level down (`h1`→`h2` … `h5`→`h6`; `h6` stays), so a README
/// can never add a second `<h1>` to a page whose own `<h1>` names the page, and its outline nests below the
/// page's (RFC 012 D-2). It runs on the **sanitized** output, so it covers raw HTML headings as well as
/// Markdown's, and it never sees anything the sanitizer let through but a well-formed tag.
///
/// The sanitizer's serializer writes `<` in text as `&lt;` (and in attribute values too) and quotes every
/// attribute value with `"`, so a `<` is always the start of a tag; the scan still ends a tag only at a `>`
/// outside a quoted value, so it does not depend on the escaping: an attribute value that looks like `<h1>`
/// is left alone.
fn demote_headings(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut copied = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let mut name = i + 1;
        if b.get(name) == Some(&b'/') {
            name += 1;
        }
        let is_heading = b.get(name) == Some(&b'h')
            && b.get(name + 1).is_some_and(|d| (b'1'..=b'5').contains(d))
            && matches!(b.get(name + 2), Some(b'>' | b' ' | b'/' | b'\t' | b'\n'));
        if is_heading {
            out.push_str(&html[copied..=name]);
            out.push((b[name + 1] + 1) as char);
            copied = name + 2;
        }
        // Skip to the end of the tag, past any `>` inside a quoted value.
        let mut quoted = false;
        i = name;
        while i < b.len() && (quoted || b[i] != b'>') {
            if b[i] == b'"' {
                quoted = !quoted;
            }
            i += 1;
        }
        i += 1;
    }
    out.push_str(&html[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_stripped() {
        let dirty = r#"<p>hi</p><script>alert('xss')</script>"#;
        let clean = sanitize_html(dirty);
        assert!(clean.contains("<p>hi</p>"));
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("alert"));
    }

    #[test]
    fn event_handlers_are_stripped() {
        let dirty = r#"<img src="x" onerror="alert(1)">"#;
        let clean = sanitize_html(dirty);
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("alert"));
    }

    #[test]
    fn javascript_urls_are_neutralized() {
        let dirty = r#"<a href="javascript:alert(1)">click</a>"#;
        let clean = sanitize_html(dirty);
        assert!(!clean.contains("javascript:"));
    }

    #[test]
    fn data_script_urls_are_neutralized() {
        let dirty = r#"<a href="data:text/html,<script>alert(1)</script>">x</a>"#;
        let clean = sanitize_html(dirty);
        assert!(!clean.contains("data:text/html"));
        assert!(!clean.contains("<script"));
    }

    #[test]
    fn markdown_renders_but_embedded_html_is_sanitized() {
        let md = "# Title\n\nSome **bold** text and a <script>alert(1)</script> attempt.";
        let out = render_markdown(md);
        assert!(out.contains("<h2>Title</h2>"), "{out}");
        assert!(out.contains("<strong>bold</strong>"));
        assert!(!out.contains("<script"));
        assert!(!out.contains("alert"));
    }

    #[test]
    fn markdown_autolink_javascript_is_not_executable_markup() {
        // A raw inline HTML anchor with a javascript: URL embedded in markdown is neutralized.
        let md = "text <a href=\"javascript:evil()\">x</a> more";
        let out = render_markdown(md);
        assert!(!out.contains("javascript:"));
    }

    #[test]
    fn headings_in_rendered_content_move_one_level_down_and_stop_at_h6() {
        let md = "# one\n\n## two\n\n### three\n\n#### four\n\n##### five\n\n###### six\n";
        assert_eq!(
            render_markdown(md),
            "<h2>one</h2>\n<h3>two</h3>\n<h4>three</h4>\n<h5>four</h5>\n<h6>five</h6>\n<h6>six</h6>\n"
        );
    }

    #[test]
    fn raw_html_headings_are_demoted_too_and_nothing_else_is_touched() {
        let out = render_markdown("<h1>raw</h1>\n\nplain <b>bold</b> and <hr> and <h1x>no</h1x>\n");
        assert!(out.contains("<h2>raw</h2>"), "{out}");
        assert!(!out.contains("<h1"), "{out}");
        assert!(out.contains("<b>bold</b>"), "{out}");
        assert!(out.contains("<hr>"), "{out}");
    }

    #[test]
    fn hostile_heading_markup_stays_inert_after_demotion() {
        let md = "# <script>alert(1)</script>t <img src=x onerror=alert(2)>\n\n\
                  <h1 onclick=\"alert(3)\" style=\"x\">a</h1>\n\n\
                  <h2 id=\"a\"><a href=\"javascript:alert(4)\">b</a></h2>\n\n\
                  <h3 title=\"</h3><h1>x\">c</h3>\n\n\
                  ## `<h1>` and &lt;h1&gt; are text\n";
        let out = render_markdown(md);
        for bad in [
            "<script",
            "alert",
            "onclick",
            "onerror",
            "javascript:",
            "style=",
        ] {
            assert!(!out.contains(bad), "{bad} survived: {out}");
        }
        assert!(!out.contains("<h1"), "{out}");
        // The attribute value that looks like tags is data (escaped by the serializer): it is kept as it is
        // and does not end the heading early.
        assert!(
            out.contains("<h4 title=\"&lt;/h3&gt;&lt;h1&gt;x\">c</h4>"),
            "{out}"
        );
        // Text that spells a tag is escaped text and stays so.
        assert!(out.contains("&lt;h1&gt;"), "{out}");
        assert!(!out.contains("<h1>"), "{out}");
    }
}
