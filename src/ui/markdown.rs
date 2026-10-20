//! A compact markdown renderer for chat messages: headings, paragraphs with
//! emphasis/code/links, lists, quotes, rules, tables and fenced code blocks
//! highlighted with syntect using a theme built from the Material palette.

use super::widgets;
use crate::theme::M3Palette;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontFamily, FontId, Frame, Margin, RichText, Stroke, Ui};
use egui_material_icons::icons::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading(u8, Vec<Span>),
    Code { lang: String, code: String },
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Quote(Vec<Block>),
    Rule,
    Table { header: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
}

/// Parse markdown into blocks.
pub fn parse(src: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut p = Builder::default();
    for ev in Parser::new_ext(src, opts) {
        p.event(ev);
    }
    p.finish()
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
    link: Option<String>,
}

enum Frame_ {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Item(Vec<Block>),
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame_>,
    spans: Vec<Span>,
    style: Style,
    heading: Option<u8>,
    code: Option<(String, String)>,
    #[allow(clippy::type_complexity)]
    table: Option<(Vec<Vec<Span>>, Vec<Vec<Vec<Span>>>, Vec<Vec<Span>>, bool)>,
}

impl Builder {
    fn blocks(&mut self) -> &mut Vec<Block> {
        if self.stack.is_empty() {
            self.stack.push(Frame_::Root(Vec::new()));
        }
        match self.stack.last_mut().expect("non-empty") {
            Frame_::Root(b) | Frame_::Quote(b) | Frame_::Item(b) => b,
            Frame_::List { items, .. } => {
                if items.is_empty() {
                    items.push(Vec::new());
                }
                items.last_mut().expect("non-empty")
            }
        }
    }

    fn flush_paragraph(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        self.blocks().push(Block::Paragraph(spans));
    }

    fn push_text(&mut self, text: &str, code: bool) {
        self.spans.push(Span {
            text: text.to_string(),
            bold: self.style.bold > 0,
            italic: self.style.italic > 0,
            strike: self.style.strike > 0,
            code,
            link: self.style.link.clone(),
        });
    }

    fn event(&mut self, ev: Event<'_>) {
        if let Some((_, code)) = &mut self.code {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, mut code) = self.code.take().expect("in code");
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    self.blocks().push(Block::Code { lang, code });
                }
                _ => {}
            }
            return;
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.flush_paragraph(),
                Tag::Heading { level, .. } => {
                    self.flush_paragraph();
                    self.heading = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        _ => 4,
                    });
                }
                Tag::BlockQuote(_) => {
                    self.flush_paragraph();
                    self.stack.push(Frame_::Quote(Vec::new()));
                }
                Tag::CodeBlock(kind) => {
                    self.flush_paragraph();
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) => l.split(|c: char| c == ',' || c.is_whitespace()).next().unwrap_or("").to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.code = Some((lang, String::new()));
                }
                Tag::List(start) => {
                    self.flush_paragraph();
                    self.stack.push(Frame_::List { start, items: Vec::new() });
                }
                Tag::Item => {
                    self.flush_paragraph();
                    self.stack.push(Frame_::Item(Vec::new()));
                }
                Tag::Emphasis => self.style.italic += 1,
                Tag::Strong => self.style.bold += 1,
                Tag::Strikethrough => self.style.strike += 1,
                Tag::Link { dest_url, .. } => self.style.link = Some(dest_url.to_string()),
                Tag::Table(_) => {
                    self.flush_paragraph();
                    self.table = Some((Vec::new(), Vec::new(), Vec::new(), true));
                }
                Tag::TableHead => {
                    if let Some(t) = &mut self.table {
                        t.3 = true;
                    }
                }
                Tag::TableRow => {
                    if let Some(t) = &mut self.table {
                        t.2.clear();
                    }
                }
                Tag::TableCell => self.spans.clear(),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => self.flush_paragraph(),
                TagEnd::Heading(_) => {
                    let spans = std::mem::take(&mut self.spans);
                    let level = self.heading.take().unwrap_or(3);
                    self.blocks().push(Block::Heading(level, spans));
                }
                TagEnd::BlockQuote(_) => {
                    self.flush_paragraph();
                    if let Some(Frame_::Quote(b)) = self.stack.pop() {
                        self.blocks().push(Block::Quote(b));
                    }
                }
                TagEnd::List(_) => {
                    self.flush_paragraph();
                    if let Some(Frame_::List { start, items }) = self.stack.pop() {
                        self.blocks().push(Block::List { start, items });
                    }
                }
                TagEnd::Item => {
                    self.flush_paragraph();
                    if let Some(Frame_::Item(b)) = self.stack.pop() {
                        if let Some(Frame_::List { items, .. }) = self.stack.last_mut() {
                            items.push(b);
                        }
                    }
                }
                TagEnd::Emphasis => self.style.italic = self.style.italic.saturating_sub(1),
                TagEnd::Strong => self.style.bold = self.style.bold.saturating_sub(1),
                TagEnd::Strikethrough => self.style.strike = self.style.strike.saturating_sub(1),
                TagEnd::Link => self.style.link = None,
                TagEnd::TableCell => {
                    let cell = std::mem::take(&mut self.spans);
                    if let Some(t) = &mut self.table {
                        t.2.push(cell);
                    }
                }
                TagEnd::TableHead => {
                    if let Some(t) = &mut self.table {
                        t.0 = std::mem::take(&mut t.2);
                        t.3 = false;
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = &mut self.table {
                        let row = std::mem::take(&mut t.2);
                        if t.3 {
                            t.0 = row;
                        } else {
                            t.1.push(row);
                        }
                    }
                }
                TagEnd::Table => {
                    if let Some((header, rows, _, _)) = self.table.take() {
                        self.blocks().push(Block::Table { header, rows });
                    }
                }
                _ => {}
            },
            Event::Text(t) => self.push_text(&t, false),
            Event::Code(t) => self.push_text(&t, true),
            Event::SoftBreak => self.push_text(" ", false),
            Event::HardBreak => self.push_text("\n", false),
            Event::Rule => {
                self.flush_paragraph();
                self.blocks().push(Block::Rule);
            }
            Event::TaskListMarker(done) => self.push_text(if done { "☑ " } else { "☐ " }, false),
            Event::Html(h) | Event::InlineHtml(h) => self.push_text(&h, false),
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush_paragraph();
        if let Some((lang, code)) = self.code.take() {
            // Unterminated fence while streaming.
            self.blocks().push(Block::Code { lang, code });
        }
        // Close anything left open (streaming).
        while self.stack.len() > 1 {
            match self.stack.pop() {
                Some(Frame_::Quote(b)) => self.blocks().push(Block::Quote(b)),
                Some(Frame_::List { start, items }) => self.blocks().push(Block::List { start, items }),
                Some(Frame_::Item(b)) => {
                    if let Some(Frame_::List { items, .. }) = self.stack.last_mut() {
                        items.push(b);
                    }
                }
                _ => {}
            }
        }
        match self.stack.pop() {
            Some(Frame_::Root(b)) => b,
            _ => Vec::new(),
        }
    }
}

// ------------------------------------------------------------------ cache

fn hash_of(t: &impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn parsed(src: &str) -> Arc<Vec<Block>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Arc<Vec<Block>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = hash_of(&src);
    if let Some(b) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return b;
    }
    let blocks = Arc::new(parse(src));
    if let Ok(mut c) = cache.lock() {
        if c.len() > 2000 {
            c.clear();
        }
        c.insert(key, blocks.clone());
    }
    blocks
}

// ----------------------------------------------------------------- render

/// Render markdown. Links open with `on_link`.
pub fn show(ui: &mut Ui, src: &str, size: f32, on_link: &mut dyn FnMut(String)) {
    let blocks = parsed(src);
    let p = widgets::palette(ui);
    ui.spacing_mut().item_spacing.y = 6.0;
    for b in blocks.iter() {
        show_block(ui, b, &p, size, on_link);
    }
}

fn show_block(ui: &mut Ui, b: &Block, p: &M3Palette, size: f32, on_link: &mut dyn FnMut(String)) {
    match b {
        Block::Paragraph(spans) => spans_label(ui, spans, p, size, p.on_surface, on_link),
        Block::Heading(level, spans) => {
            let s = match level {
                1 => size * 1.6,
                2 => size * 1.35,
                3 => size * 1.15,
                _ => size * 1.05,
            };
            ui.add_space(4.0);
            spans_label(ui, spans, p, s, p.on_surface, on_link);
        }
        Block::Code { lang, code } => code_block(ui, lang, code, p, size),
        Block::List { start, items } => {
            for (i, item) in items.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    let marker = match start {
                        Some(n) => format!("{}.", n + i as u64),
                        None => "•".into(),
                    };
                    ui.add_sized(
                        [22.0, size + 4.0],
                        egui::Label::new(RichText::new(marker).size(size).color(p.primary)),
                    );
                    ui.vertical(|ui| {
                        for b in item {
                            show_block(ui, b, p, size, on_link);
                        }
                    });
                });
            }
        }
        Block::Quote(inner) => {
            Frame::new()
                .stroke(Stroke::NONE)
                .inner_margin(Margin { left: 12, right: 4, top: 2, bottom: 2 })
                .show(ui, |ui| {
                    let r = ui.max_rect();
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(r.min - egui::vec2(12.0, 0.0), egui::vec2(3.0, r.height().max(size))),
                        CornerRadius::same(2),
                        p.outline_variant,
                    );
                    for b in inner {
                        show_block(ui, b, p, size, on_link);
                    }
                });
        }
        Block::Rule => {
            ui.separator();
        }
        Block::Table { header, rows } => {
            Frame::new()
                .stroke(Stroke::new(1.0, p.outline_variant))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    egui::Grid::new(ui.next_auto_id()).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                        for cell in header {
                            let bold: Vec<Span> = cell.iter().map(|s| Span { bold: true, ..s.clone() }).collect();
                            spans_label(ui, &bold, p, size, p.on_surface, on_link);
                        }
                        ui.end_row();
                        for row in rows {
                            for cell in row {
                                spans_label(ui, cell, p, size, p.on_surface, on_link);
                            }
                            ui.end_row();
                        }
                    });
                });
        }
    }
}

/// A paragraph as one wrapped galley; links are found from the hovered
/// character so they can be clicked.
pub fn spans_label(ui: &mut Ui, spans: &[Span], p: &M3Palette, size: f32, color: Color32, on_link: &mut dyn FnMut(String)) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut chars = 0usize;
    for s in spans {
        let family = if s.code {
            FontFamily::Monospace
        } else if s.bold {
            crate::theme::style::medium_family()
        } else {
            FontFamily::Proportional
        };
        let mut f = TextFormat {
            font_id: FontId::new(if s.code { size * 0.92 } else { size }, family),
            color: if s.link.is_some() { p.primary } else if s.code { p.on_secondary_container } else { color },
            italics: s.italic,
            ..Default::default()
        };
        if s.code {
            f.background = p.secondary_container.gamma_multiply(0.6);
        }
        if s.strike {
            f.strikethrough = Stroke::new(1.0, color);
        }
        if s.link.is_some() {
            f.underline = Stroke::new(1.0, p.primary.gamma_multiply(0.6));
        }
        let n = s.text.chars().count();
        if let Some(l) = &s.link {
            links.push((chars..chars + n, l.clone()));
        }
        chars += n;
        job.append(&s.text, 0.0, f);
    }
    let galley = ui.painter().layout_job(job);
    let sense = if links.is_empty() { egui::Sense::hover() } else { egui::Sense::click() };
    let resp = ui.add(egui::Label::new(galley.clone()).sense(sense).selectable(links.is_empty()));
    if !links.is_empty() {
        if let Some(pos) = resp.hover_pos() {
            let idx = galley.cursor_from_pos(pos - resp.rect.min).index.0;
            if let Some((_, url)) = links.iter().find(|(r, _)| r.contains(&idx)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    on_link(url.clone());
                }
                resp.on_hover_text(url);
            }
        }
    }
}

fn code_block(ui: &mut Ui, lang: &str, code: &str, p: &M3Palette, size: f32) {
    Frame::new()
        .fill(p.surface_container_highest)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin { left: 14, right: 8, top: 6, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(if lang.is_empty() { "code" } else { lang }).size(12.0).color(p.on_surface_variant));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button_sized(ui, ICON_CONTENT_COPY, "Copy", true, 28.0, false).clicked() {
                        ui.ctx().copy_text(code.to_string());
                    }
                });
            });
            egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                let job = highlight(code, lang, p, size * 0.9);
                ui.add(egui::Label::new(job).extend());
            });
        });
}

// ------------------------------------------------------------ highlighting

struct Highlighter {
    syntaxes: syntect::parsing::SyntaxSet,
}

fn highlighter() -> &'static Highlighter {
    static H: OnceLock<Highlighter> = OnceLock::new();
    H.get_or_init(|| Highlighter { syntaxes: syntect::parsing::SyntaxSet::load_defaults_newlines() })
}

/// A syntect theme whose colors are Material roles, so code matches the app.
fn theme(p: &M3Palette) -> syntect::highlighting::Theme {
    use std::str::FromStr;
    use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
    let c = |c: Color32| Color { r: c.r(), g: c.g(), b: c.b(), a: 255 };
    let item = |scope: &str, color: Color32, italic: bool| ThemeItem {
        scope: ScopeSelectors::from_str(scope).unwrap_or_default(),
        style: StyleModifier {
            foreground: Some(c(color)),
            background: None,
            font_style: italic.then_some(syntect::highlighting::FontStyle::ITALIC),
        },
    };
    let dark = p.is_dark();
    let mix = |a: Color32, b: Color32| crate::theme::palette::blend(a, b, if dark { 0.15 } else { 0.0 });
    Theme {
        name: Some("material".into()),
        author: None,
        settings: ThemeSettings { foreground: Some(c(p.on_surface)), ..Default::default() },
        scopes: vec![
            item("comment, punctuation.definition.comment", p.outline, true),
            item("string, string.quoted, markup.raw", mix(p.tertiary, Color32::WHITE), false),
            item("constant.numeric, constant.language, constant.character", p.secondary, false),
            item("keyword, storage, storage.type, keyword.control", p.primary, false),
            item("entity.name.function, support.function, meta.function-call", mix(p.primary, p.tertiary), false),
            item("entity.name.type, support.type, entity.name.class, support.class", p.secondary, false),
            item("variable.parameter, variable.language", p.tertiary, true),
            item("entity.name.tag, meta.tag", p.primary, false),
            item("entity.other.attribute-name", p.secondary, false),
            item("markup.inserted", Color32::from_rgb(0x4C, 0xAF, 0x50), false),
            item("markup.deleted", p.error, false),
            item("markup.heading", p.primary, false),
        ],
    }
}

pub fn highlight(code: &str, lang: &str, p: &M3Palette, size: f32) -> LayoutJob {
    static CACHE: OnceLock<Mutex<HashMap<u64, LayoutJob>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = hash_of(&(code, lang, p.primary.to_array(), p.on_surface.to_array(), (size * 10.0) as u32));
    if let Some(j) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return j;
    }
    let mut job = LayoutJob::default();
    let font = FontId::monospace(size);
    let h = highlighter();
    let syntax = h
        .syntaxes
        .find_syntax_by_token(lang)
        .or_else(|| h.syntaxes.find_syntax_by_extension(lang))
        .unwrap_or_else(|| h.syntaxes.find_syntax_plain_text());
    let theme = theme(p);
    let mut hl = syntect::easy::HighlightLines::new(syntax, &theme);
    // Very long blocks are shown plain to keep frames fast.
    if code.len() > 60_000 {
        job.append(code, 0.0, TextFormat::simple(font, p.on_surface));
    } else {
        for line in syntect::util::LinesWithEndings::from(code) {
            match hl.highlight_line(line, &h.syntaxes) {
                Ok(ranges) => {
                    for (style, text) in ranges {
                        let fg = style.foreground;
                        let mut f = TextFormat::simple(font.clone(), Color32::from_rgb(fg.r, fg.g, fg.b));
                        f.italics = style.font_style.contains(syntect::highlighting::FontStyle::ITALIC);
                        job.append(text, 0.0, f);
                    }
                }
                Err(_) => job.append(line, 0.0, TextFormat::simple(font.clone(), p.on_surface)),
            }
        }
    }
    if let Ok(mut c) = cache.lock() {
        if c.len() > 500 {
            c.clear();
        }
        c.insert(key, job.clone());
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_markdown() {
        let b = parse("# Title\n\nSome **bold** and `code` with [a link](https://x.y).\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(matches!(&b[0], Block::Heading(1, s) if s[0].text == "Title"));
        let Block::Paragraph(spans) = &b[1] else { panic!("{:?}", b[1]) };
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        assert!(spans.iter().any(|s| s.code && s.text == "code"));
        assert!(spans.iter().any(|s| s.link.as_deref() == Some("https://x.y")));
        assert!(matches!(&b[2], Block::List { start: None, items } if items.len() == 2));
        assert!(matches!(&b[3], Block::Code { lang, code } if lang == "rust" && code == "fn main() {}"));
        assert!(matches!(&b[4], Block::Quote(_)));
        assert!(matches!(&b[5], Block::Table { header, rows } if header.len() == 2 && rows.len() == 1));
    }

    #[test]
    fn unterminated_fence_while_streaming() {
        let b = parse("Here:\n\n```py\nprint(1)\n");
        assert!(matches!(&b[1], Block::Code { lang, code } if lang == "py" && code.contains("print")));
    }

    #[test]
    fn highlights_with_palette() {
        let p = M3Palette::from_seed("#6750A4", true);
        let job = highlight("fn main() { let x = 1; }", "rust", &p, 13.0);
        assert!(job.sections.len() > 3, "tokens get separate colors");
        assert!(job.sections.iter().any(|s| s.format.color == p.primary));
    }
}
