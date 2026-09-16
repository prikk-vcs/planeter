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
    sanitize_html(&raw)
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
        assert!(out.contains("<h1>"));
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
}
