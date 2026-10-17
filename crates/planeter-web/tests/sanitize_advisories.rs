//! Regression tests for the sanitizer shapes behind RUSTSEC-2021-0157 and RUSTSEC-2021-0158 (ammonia)
//! and their neighbours (supply-chain handoff v1, T2). They pin the **behaviour of planeter's
//! configuration** (`ammonia::clean`, defaults): they must hold on the locked version before and after
//! the lock update, because under this configuration the advisory conditions are not reachable. They
//! assert on the output, never on the library version.

use planeter_web::{render_markdown, sanitize_html};

fn assert_inert(out: &str) {
    for needle in [
        "javascript:",
        "onerror",
        "onload",
        "<script",
        "<svg",
        "<math",
        "<style",
        "animate",
        "style=",
        "alert",
    ] {
        assert!(
            !out.to_ascii_lowercase().contains(needle),
            "{needle:?} survived: {out}"
        );
    }
}

#[test]
fn style_attribute_with_a_javascript_url_leaves_no_style() {
    // RUSTSEC-2021-0157 shape.
    let out = sanitize_html(r#"<p style="background:url('javascript:alert(1)')">kept</p>"#);
    assert!(out.contains("kept"), "text must survive: {out}");
    assert_inert(&out);
}

#[test]
fn style_attribute_with_an_empty_url_function_leaves_no_style() {
    // RUSTSEC-2021-0157 shape (`url("")`).
    let out = sanitize_html(r#"<div style='background:url("");color:red'>kept</div>"#);
    assert!(out.contains("kept"), "text must survive: {out}");
    assert_inert(&out);
}

#[test]
fn svg_animate_motion_leaves_nothing() {
    // RUSTSEC-2021-0158 shape.
    let out = sanitize_html(
        r#"<svg><a><animateMotion path="M0,0" xlink:href="javascript:alert(1)"/><text>t</text></a></svg>"#,
    );
    assert_inert(&out);
}

#[test]
fn svg_animate_href_javascript_values_leaves_nothing() {
    let out = sanitize_html(
        r##"<svg><a href="#"><animate attributeName="href" values="javascript:alert(1)"/><text>x</text></a></svg>"##,
    );
    assert_inert(&out);
}

#[test]
fn svg_animate_transform_and_color_leave_nothing() {
    let out = sanitize_html(
        r#"<svg><animateTransform attributeName="transform" type="rotate"/><animateColor attributeName="fill"/></svg>after"#,
    );
    assert!(
        out.contains("after"),
        "text outside the svg survives: {out}"
    );
    assert_inert(&out);
}

#[test]
fn math_mtext_style_img_onerror_leaves_nothing() {
    // The mXSS neighbour of the two advisories.
    let out = sanitize_html(r#"<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>"#);
    assert_inert(&out);
}

#[test]
fn anchor_javascript_href_keeps_text_and_loses_href() {
    let out = sanitize_html(r#"<a href="javascript:alert(1)">click me</a>"#);
    assert!(out.contains("click me"), "text must survive: {out}");
    assert!(!out.contains("href"), "the href must be dropped: {out}");
    assert_inert(&out);
}

#[test]
fn the_same_shapes_are_inert_through_markdown() {
    // Markdown passes raw HTML through to the sanitizer; the rendered result must be just as inert.
    let md = "para\n\n<svg><animate attributeName=\"href\" values=\"javascript:alert(1)\"/></svg>\n\n\
              <p style=\"background:url('javascript:alert(1)')\">kept</p>\n";
    let out = render_markdown(md);
    assert!(out.contains("kept") && out.contains("para"), "{out}");
    assert_inert(&out);
}

// ---- RUSTSEC-2026-0373: a mixed-case CSS `URL(` function (IS-38) ----------------------------------

#[test]
fn a_mixed_case_css_url_function_in_a_style_attribute_leaves_nothing() {
    // The advisory's shape: `filter_style_properties` missed a non-lowercase `url(`. planeter's default
    // configuration drops the `style` attribute whole, so none of these may survive.
    for call in [
        r#"URL("https://evil.example/x")"#,
        r#"Url('https://evil.example/x')"#,
        r#"uRl(https://evil.example/x)"#,
        r#"URL( "https://evil.example/x" )"#,
    ] {
        let dirty = format!(r#"<p style="background:{call};color:red">kept</p>"#);
        let out = sanitize_html(&dirty);
        assert!(out.contains("kept"), "text must survive: {out}");
        assert_inert(&out);
        assert!(
            !out.to_ascii_lowercase().contains("evil.example"),
            "{call}: {out}"
        );
        assert!(!out.to_ascii_lowercase().contains("url("), "{call}: {out}");
    }
}

#[test]
fn a_mixed_case_css_url_function_in_a_style_element_leaves_nothing() {
    for call in [
        r#"URL("https://evil.example/x")"#,
        r#"Url(https://evil.example/x)"#,
    ] {
        let dirty = format!("<style>p{{background:{call}}}</style><p>kept</p>");
        let out = sanitize_html(&dirty);
        assert!(out.contains("kept"), "text must survive: {out}");
        assert_inert(&out);
        assert!(
            !out.to_ascii_lowercase().contains("evil.example"),
            "{call}: {out}"
        );
        assert!(!out.to_ascii_lowercase().contains("url("), "{call}: {out}");
    }
}

#[test]
fn the_mixed_case_url_shapes_are_inert_through_markdown() {
    let md = "para\n\n<p style=\"background:URL('https://evil.example/x')\">kept</p>\n\n\
              <style>p{background:Url(https://evil.example/x)}</style>\n";
    let out = render_markdown(md);
    assert!(out.contains("kept") && out.contains("para"), "{out}");
    assert_inert(&out);
    assert!(!out.to_ascii_lowercase().contains("evil.example"), "{out}");
}
