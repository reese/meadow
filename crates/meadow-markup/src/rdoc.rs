//! A conservative `RDoc` markup subset.
//!
//! Supported constructs: paragraphs, `=` headings, bullet and numbered lists,
//! `+code+`, `<tt>`, `*bold*`, `_em_`, verbatim indented blocks, and labeled
//! lists (`[term]` / `term::`). Unsupported `RDoc` constructs degrade to text.

use std::fmt::Write;

use crate::Markup;

/// `RDoc` markup rendering.
#[derive(Debug, Clone, Copy)]
pub struct RDoc {
    /// Whether raw HTML should be escaped. `RDoc` output always escapes text.
    pub sanitize: bool,
}

impl RDoc {
    /// Creates an `RDoc` provider.
    #[must_use]
    pub const fn new(sanitize: bool) -> Self {
        Self { sanitize }
    }
}

impl Default for RDoc {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Markup for RDoc {
    fn render(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let mut output = String::new();
        let mut paragraph: Vec<&str> = Vec::new();
        let mut list: Option<&'static str> = None;
        let mut definition = false;
        let mut index = 0;

        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.trim();

            if trimmed.is_empty() {
                flush_paragraph(&mut output, &mut paragraph);
                close_list(&mut output, &mut list);
                close_definition(&mut output, &mut definition);
                index += 1;
                continue;
            }

            if let Some((level, title)) = heading(trimmed) {
                flush_paragraph(&mut output, &mut paragraph);
                close_list(&mut output, &mut list);
                close_definition(&mut output, &mut definition);
                let _ = write!(output, "<h{level}>{}</h{level}>", inline(title));
                index += 1;
                continue;
            }

            if line.starts_with(' ') || line.starts_with('\t') {
                flush_paragraph(&mut output, &mut paragraph);
                close_list(&mut output, &mut list);
                close_definition(&mut output, &mut definition);
                let mut verbatim = Vec::new();
                while index < lines.len()
                    && (lines[index].starts_with(' ')
                        || lines[index].starts_with('\t')
                        || lines[index].trim().is_empty())
                {
                    verbatim.push(lines[index]);
                    index += 1;
                }
                output.push_str("<pre><code>");
                output.push_str(&escape(&dedent(&verbatim)));
                output.push_str("</code></pre>");
                continue;
            }

            if let Some((kind, content)) = list_item(line) {
                flush_paragraph(&mut output, &mut paragraph);
                close_definition(&mut output, &mut definition);
                if list != Some(kind) {
                    close_list(&mut output, &mut list);
                    output.push_str(if kind == "ol" { "<ol>" } else { "<ul>" });
                    list = Some(kind);
                }
                output.push_str("<li>");
                output.push_str(&inline(content));
                output.push_str("</li>");
                index += 1;
                continue;
            }

            if let Some((term, description)) = labeled(trimmed) {
                flush_paragraph(&mut output, &mut paragraph);
                close_list(&mut output, &mut list);
                if !definition {
                    output.push_str("<dl>");
                    definition = true;
                }
                output.push_str("<dt>");
                output.push_str(&inline(&term));
                output.push_str("</dt><dd>");
                output.push_str(&inline(&description));
                output.push_str("</dd>");
                index += 1;
                continue;
            }

            close_list(&mut output, &mut list);
            close_definition(&mut output, &mut definition);
            paragraph.push(trimmed);
            index += 1;
        }

        flush_paragraph(&mut output, &mut paragraph);
        close_list(&mut output, &mut list);
        close_definition(&mut output, &mut definition);
        output
    }
}

fn flush_paragraph(output: &mut String, paragraph: &mut Vec<&str>) {
    if paragraph.is_empty() {
        return;
    }
    output.push_str("<p>");
    output.push_str(&inline(&paragraph.join(" ")));
    output.push_str("</p>");
    paragraph.clear();
}

fn close_list(output: &mut String, list: &mut Option<&'static str>) {
    if let Some(kind) = list.take() {
        output.push_str(if kind == "ol" { "</ol>" } else { "</ul>" });
    }
}

fn close_definition(output: &mut String, definition: &mut bool) {
    if *definition {
        output.push_str("</dl>");
        *definition = false;
    }
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let count = line
        .chars()
        .take_while(|character| *character == '=')
        .count();
    if count == 0 || count > 6 {
        return None;
    }
    let title = line[count..].trim_start();
    (!title.is_empty() && line.as_bytes().get(count) == Some(&b' ')).then_some((count, title))
}

fn list_item(line: &str) -> Option<(&'static str, &str)> {
    let trimmed = line.trim_start();
    if let Some(content) = trimmed.strip_prefix("* ") {
        return Some(("ul", content));
    }
    if let Some(content) = trimmed.strip_prefix("- ") {
        return Some(("ul", content));
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &trimmed[digits..];
        if let Some(content) = rest.strip_prefix(". ") {
            return Some(("ol", content));
        }
    }
    None
}

fn labeled(line: &str) -> Option<(String, String)> {
    if let Some(rest) = line.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let term = rest[..end].trim();
        let description = rest[end + 1..].trim_start();
        return Some((term.to_string(), description.to_string()));
    }
    if let Some(position) = line.find("::")
        && position > 0
    {
        let term = line[..position].trim();
        let description = line[position + 2..].trim_start();
        if !term.is_empty() && !term.contains(' ') {
            return Some((term.to_string(), description.to_string()));
        }
    }
    None
}

fn dedent(lines: &[&str]) -> String {
    let indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|line| line.get(indent..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn inline(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut output = String::new();
    let mut index = 0;

    while index < characters.len() {
        if characters[index] == '['
            && let Some((label, url, end)) = markdown_link(&characters, index)
        {
            output.push_str("<a href=\"");
            output.push_str(&escape(&url));
            output.push_str("\">");
            output.push_str(&inline(&label));
            output.push_str("</a>");
            index = end;
            continue;
        }

        if starts_with(&characters, index, "<tt>")
            && let Some(end) = find(&characters, index + 4, "</tt>")
        {
            let content: String = characters[index + 4..end].iter().collect();
            output.push_str("<code>");
            output.push_str(&escape(&content));
            output.push_str("</code>");
            index = end + 5;
            continue;
        }

        match characters[index] {
            '+' | '*' | '_' => {
                let marker = characters[index];
                if let Some(end) = find(&characters, index + 1, &marker.to_string()) {
                    let content: String = characters[index + 1..end].iter().collect();
                    let tag = match marker {
                        '*' => "strong",
                        '_' => "em",
                        _ => "code",
                    };
                    output.push('<');
                    output.push_str(tag);
                    output.push('>');
                    output.push_str(&if marker == '+' {
                        escape(&content)
                    } else {
                        inline(&content)
                    });
                    output.push_str("</");
                    output.push_str(tag);
                    output.push('>');
                    index = end + 1;
                    continue;
                }
                output.push_str(&escape_char(marker));
                index += 1;
            }
            character => {
                output.push_str(&escape_char(character));
                index += 1;
            }
        }
    }
    output
}

fn markdown_link(characters: &[char], start: usize) -> Option<(String, String, usize)> {
    let label_end = find(characters, start + 1, "]")?;
    if characters.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let url_end = find(characters, label_end + 2, ")")?;
    Some((
        characters[start + 1..label_end].iter().collect(),
        characters[label_end + 2..url_end].iter().collect(),
        url_end + 1,
    ))
}

fn starts_with(characters: &[char], start: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, character)| characters.get(start + offset) == Some(&character))
}

fn find(characters: &[char], start: usize, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    let needle: Vec<char> = needle.chars().collect();
    let mut index = start;
    while index + needle.len() <= characters.len() {
        if characters[index..index + needle.len()] == needle[..] {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        output.push_str(&escape_char(character));
    }
    output
}

fn escape_char(character: char) -> String {
    match character {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '"' => "&quot;".to_string(),
        '\'' => "&#39;".to_string(),
        other => other.to_string(),
    }
}
