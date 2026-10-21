//! Code highlighting with syntect's grammars, plugged into material-3-ui's
//! Markdown renderer. syntect only classifies text into the shared token
//! kinds (each kind is encoded as a color of a fixed "token theme"); the
//! kinds become Material colors when painting, so code looks the same as
//! in the other Material 3 apps and follows wallpaper changes at once.

use egui::text::LayoutJob;
use material_3_ui::code::{Span, Tok, TOKEN_SCOPES};
use material_3_ui::theme::M3Palette;
use std::sync::OnceLock;
use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::SyntaxSet;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Scopes [`TOKEN_SCOPES`] misses with syntect's grammars: Rust's `self`
/// is `variable.language.rust`, not `variable.language.self`.
const EXTRA_SCOPES: &[(&str, Tok)] = &[("variable.language", Tok::Keyword)];

/// [`TOKEN_SCOPES`] as a syntect theme: a token's kind is its red channel.
fn token_theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        use std::str::FromStr;
        let kind = |tok: Tok| Color { r: tok as u8, g: 0, b: 0, a: 255 };
        Theme {
            name: Some("m3-tokens".into()),
            author: None,
            settings: ThemeSettings { foreground: Some(kind(Tok::Plain)), ..Default::default() },
            scopes: EXTRA_SCOPES
                .iter()
                .chain(TOKEN_SCOPES)
                .map(|(scope, tok)| ThemeItem {
                    scope: ScopeSelectors::from_str(scope).unwrap_or_default(),
                    style: StyleModifier { foreground: Some(kind(*tok)), background: None, font_style: None },
                })
                .collect(),
        }
    })
}

/// Token spans for `code` in `lang` (a fence name or file extension), one
/// `Vec` per line, in chars. Unknown languages come back plain.
pub fn spans(code: &str, lang: &str) -> Vec<Vec<Span>> {
    let set = syntaxes();
    let syntax = set
        .find_syntax_by_token(lang)
        .or_else(|| set.find_syntax_by_extension(lang))
        .unwrap_or_else(|| set.find_syntax_plain_text());
    let mut hl = syntect::easy::HighlightLines::new(syntax, token_theme());
    syntect::util::LinesWithEndings::from(code)
        .map(|line| {
            let Ok(ranges) = hl.highlight_line(line, set) else { return Vec::new() };
            let mut out: Vec<Span> = Vec::new();
            let mut at = 0;
            for (style, text) in ranges {
                let (start, tok) = (at, Tok::from_u8(style.foreground.r));
                at += text.chars().count();
                match out.last_mut() {
                    _ if tok == Tok::Plain => {}
                    // A string's quotes and contents are one span.
                    Some(last) if last.tok == tok && last.end == start => last.end = at,
                    _ => out.push(Span { start, end: at, tok }),
                }
            }
            out
        })
        .collect()
}

/// The plug-in for [`material_3_ui::markdown::show_with`].
pub fn highlighter() -> &'static material_3_ui::markdown::Highlighter {
    &spans
}

/// A highlighted code layout (cached), for tool inputs and outputs.
pub fn highlight(code: &str, lang: &str, p: &M3Palette, size: f32) -> LayoutJob {
    material_3_ui::markdown::highlight(code, lang, p, size, Some(highlighter()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_with_syntect_grammars() {
        let lines = spans("fn main() {\n    let s = \"hi\"; // done\n}\n", "rust");
        assert_eq!(lines.len(), 3);
        let kinds: Vec<Tok> = lines.iter().flatten().map(|s| s.tok).collect();
        for k in [Tok::Keyword, Tok::String, Tok::Comment] {
            assert!(kinds.contains(&k), "{k:?} in {kinds:?}");
        }
        let line = spans("self.x", "rust").remove(0);
        assert_eq!(line[0], Span { start: 0, end: 4, tok: Tok::Keyword }, "self is a keyword");
        // Char offsets, not bytes.
        let line = spans("let ü = \"ö\";", "rs").remove(0);
        let s = line.iter().find(|s| s.tok == Tok::String).unwrap();
        assert_eq!((s.start, s.end), (8, 11));
        // Diffs keep their added/removed colors.
        let diff: Vec<Tok> = spans("+added\n-removed\n", "diff").into_iter().flatten().map(|s| s.tok).collect();
        assert!(diff.contains(&Tok::Inserted) && diff.contains(&Tok::Deleted), "{diff:?}");
        assert!(spans("just words", "no-such-language").iter().all(Vec::is_empty));
    }

    #[test]
    fn colors_come_from_the_palette() {
        let p = M3Palette::from_seed("#6750A4", true);
        let job = highlight("fn main() { let x = 1; }", "rust", &p, 13.0);
        assert_eq!(job.text, "fn main() { let x = 1; }");
        assert!(job.sections.len() > 3, "tokens get separate colors");
        assert!(job.sections.iter().any(|s| s.format.color == Tok::Keyword.color(&p)));
    }
}
