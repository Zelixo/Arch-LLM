use gtk4 as gtk;
use gtk::glib;
use pulldown_cmark::{Parser, Options, Tag, TagEnd, Event};

pub fn normalize_url(s: &str) -> String {
    let mut s = s.trim().to_string();
    if !s.starts_with("http://") && !s.starts_with("https://") {
        s = format!("http://{}", s);
    }
    s
}

pub enum MarkdownBlock {
    Text(String),
    Code(String, String), // (language, code)
}

pub fn parse_markdown(markdown: &str) -> Vec<MarkdownBlock> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);
    
    let mut blocks = Vec::new();
    let mut current_text = String::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut current_code = String::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::CodeBlock(kind) => {
                    // Flush text
                    if !current_text.is_empty() {
                        blocks.push(MarkdownBlock::Text(current_text.clone()));
                        current_text.clear();
                    }
                    in_code_block = true;
                    code_lang = match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                        _ => String::new(),
                    };
                }
                Tag::Strong => current_text.push_str("<b>"),
                Tag::Emphasis => current_text.push_str("<i>"),
                Tag::Strikethrough => current_text.push_str("<s>"),
                Tag::BlockQuote(_) => current_text.push_str("<blockquote>"),
                Tag::Heading { level, .. } => {
                    let size = match level {
                        pulldown_cmark::HeadingLevel::H1 => "xx-large",
                        pulldown_cmark::HeadingLevel::H2 => "x-large",
                        _ => "large",
                    };
                    current_text.push_str(&format!("\n<span font_size=\"{}\" weight=\"bold\">", size));
                }
                Tag::Link { .. } => current_text.push_str("<u>"),
                Tag::Item => current_text.push_str("  • "),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    blocks.push(MarkdownBlock::Code(code_lang.clone(), current_code.trim().to_string()));
                    current_code.clear();
                    code_lang.clear();
                }
                TagEnd::Strong => current_text.push_str("</b>"),
                TagEnd::Emphasis => current_text.push_str("<i>"),
                TagEnd::Strikethrough => current_text.push_str("</s>"),
                TagEnd::Heading(_) => current_text.push_str("</span>\n"),
                TagEnd::BlockQuote(_) => current_text.push_str("</blockquote>\n"),
                TagEnd::Link => current_text.push_str("</u>"),
                TagEnd::Item => current_text.push_str("\n"),
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    current_code.push_str(&text);
                } else {
                    current_text.push_str(&glib::markup_escape_text(&text));
                }
            },
            Event::Code(code) => {
                if in_code_block {
                    current_code.push_str(&code);
                } else {
                    current_text.push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&code)));
                }
            },
            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    current_code.push('\n');
                } else {
                    current_text.push('\n');
                }
            },
            Event::Rule => current_text.push_str("\n───────────────────\n"),
            _ => {}
        }
    }
    
    if !current_text.is_empty() {
        blocks.push(MarkdownBlock::Text(current_text));
    }
    
    blocks
}

pub fn markdown_to_pango(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);
    let mut pango_markup = String::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Strong => pango_markup.push_str("<b>"),
                Tag::Emphasis => pango_markup.push_str("<i>"),
                Tag::Strikethrough => pango_markup.push_str("<s>"),
                Tag::CodeBlock(_) => pango_markup.push_str("\n<tt>"),
                Tag::BlockQuote(_) => pango_markup.push_str("<blockquote>"),
                Tag::Heading { level, .. } => {
                    let size = match level {
                        pulldown_cmark::HeadingLevel::H1 => "xx-large",
                        pulldown_cmark::HeadingLevel::H2 => "x-large",
                        _ => "large",
                    };
                    pango_markup.push_str(&format!("\n<span font_size=\"{}\" weight=\"bold\">", size));
                }
                Tag::Link { .. } => pango_markup.push_str("<u>"),
                Tag::Item => pango_markup.push_str("  • "),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Strong => pango_markup.push_str("</b>"),
                TagEnd::Emphasis => pango_markup.push_str("<i>"),
                TagEnd::Strikethrough => pango_markup.push_str("</s>"),
                TagEnd::CodeBlock => pango_markup.push_str("</tt>\n"),
                TagEnd::Heading(_) => pango_markup.push_str("</span>\n"),
                TagEnd::BlockQuote(_) => pango_markup.push_str("</blockquote>\n"),
                TagEnd::Link => pango_markup.push_str("</u>"),
                TagEnd::Item => pango_markup.push_str("\n"),
                _ => {}
            },
            Event::Text(text) => pango_markup.push_str(&glib::markup_escape_text(&text)),
            Event::Code(code) => pango_markup.push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&code))),
            Event::SoftBreak | Event::HardBreak => pango_markup.push('\n'),
            Event::Rule => pango_markup.push_str("\n───────────────────\n"),
            _ => {}
        }
    }
    pango_markup
}