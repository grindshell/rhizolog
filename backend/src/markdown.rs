//! Rendering page bodies to HTML.
//!
//! Rendering is opt-in: the API serves raw markdown by default, because that is
//! what an agent can reason about and edit. HTML is for the browser.

use comrak::Options;

/// Render a markdown body to HTML.
///
/// Raw HTML in the source is **escaped, not passed through**. Rhizowiki is
/// single-user and loopback-bound, so this is not guarding against a hostile
/// author — but page content arrives over an API that agents write to, and
/// rendering `<script>` from an indirect source into the dashboard is the kind
/// of thing that is very hard to notice and very easy to avoid. Turning this on
/// should be a deliberate, separate decision.
pub fn render(markdown: &str) -> String {
    comrak::markdown_to_html(markdown, &options())
}

/// Fields are set individually rather than through a struct literal because
/// comrak's option types are `non_exhaustive` and have been renamed across
/// releases; touching only the fields we care about survives both.
fn options() -> Options<'static> {
    let mut options = Options::default();

    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;

    // `options.render.unsafe_` is deliberately left at its default of false.
    // See the note on `render`.

    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ordinary_markdown() {
        let html = render("# Heading\n\nSome *emphasis* and a [link](https://example.com).\n");

        assert!(html.contains("<h1>Heading</h1>"));
        assert!(html.contains("<em>emphasis</em>"));
        assert!(html.contains(r#"<a href="https://example.com">link</a>"#));
    }

    #[test]
    fn renders_gfm_extensions() {
        assert!(render("~~gone~~").contains("<del>gone</del>"));
        assert!(render("| a | b |\n|---|---|\n| 1 | 2 |\n").contains("<table>"));
        assert!(render("- [x] done\n").contains("checked"));
        assert!(render("Visit https://example.com today").contains("<a href="));
    }

    /// The whole point of leaving comrak's `unsafe_` option off.
    ///
    /// Note what comrak actually does here: with `unsafe_` off a raw HTML block
    /// is *dropped* and replaced by a comment, not escaped into visible text.
    /// The security property is the same, but the rendered page will not show
    /// the markup either, which is worth knowing before someone reports it as a
    /// bug.
    #[test]
    fn raw_html_is_not_passed_through() {
        let html = render("<script>alert('xss')</script>\n");

        assert!(
            !html.contains("<script"),
            "raw HTML was passed through: {html}"
        );
        assert!(
            html.contains("raw HTML omitted"),
            "expected comrak's omission marker, got {html}"
        );
    }

    #[test]
    fn inline_html_is_escaped_too() {
        let html = render("Text with <img src=x onerror=alert(1)> inline.\n");

        assert!(
            !html.contains("<img"),
            "inline HTML was passed through: {html}"
        );
    }

    /// A javascript: URL must not survive as a clickable link.
    #[test]
    fn dangerous_link_schemes_are_neutralised() {
        let html = render("[click me](javascript:alert(1))\n");

        assert!(
            !html.contains("href=\"javascript:"),
            "javascript: URL survived: {html}"
        );
    }

    #[test]
    fn an_empty_body_renders_to_nothing() {
        assert_eq!(render("").trim(), "");
    }
}
