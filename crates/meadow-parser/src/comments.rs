//! Comment collection, grouping, and attachment.

use ruby_prism::{CommentType, ParseResult};

use crate::event::{CommentEvent, Docstring};
use crate::{SourceLocation, SourceText};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentKind {
    Inline,
    EmbDoc,
}

struct CommentGroup {
    kind: CommentKind,
    text: Vec<u8>,
    location: SourceLocation,
    used: bool,
}

/// A set of comment blocks for one file, with support for attaching a block to
/// the next code object and reporting the blocks that were never attached.
pub(crate) struct CommentIndex {
    groups: Vec<CommentGroup>,
}

impl CommentIndex {
    /// Builds grouped comment blocks from a Prism parse result.
    pub(crate) fn from_parse(result: &ParseResult<'_>) -> Self {
        let mut groups: Vec<CommentGroup> = Vec::new();

        for comment in result.comments() {
            let location = comment.location();
            let start_line = u32::try_from(location.start_line().max(0)).unwrap_or(0);
            let end_line = u32::try_from(location.end_line().max(0)).unwrap_or(0);
            let raw = comment.text();
            let kind = match comment.type_() {
                CommentType::InlineComment => CommentKind::Inline,
                CommentType::EmbDocComment => CommentKind::EmbDoc,
            };

            if is_ignored(kind, raw, start_line) {
                continue;
            }

            let text = match kind {
                CommentKind::Inline => strip_inline(raw),
                CommentKind::EmbDoc => strip_embdoc(raw),
            };

            let source_location = SourceLocation {
                start: location.start(),
                end: location.end(),
                start_line,
                end_line,
            };

            if let Some(last) = groups.last_mut()
                && last.kind == kind
                && start_line == last.location.end_line + 1
            {
                last.text.push(b'\n');
                last.text.extend_from_slice(&text);
                last.location.end = source_location.end;
                last.location.end_line = source_location.end_line;
                continue;
            }

            groups.push(CommentGroup {
                kind,
                text,
                location: source_location,
                used: false,
            });
        }

        Self { groups }
    }

    /// Attaches the closest comment block immediately preceding `node`, or a
    /// trailing same-line comment when no preceding block exists.
    pub(crate) fn take(&mut self, node: &SourceLocation) -> Option<Docstring> {
        let preceding = self
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                !group.used
                    && group.location.start < node.start
                    && group.location.end <= node.start
                    && node.start_line >= group.location.end_line
                    && node.start_line <= group.location.end_line.saturating_add(1)
            })
            .max_by_key(|(_, group)| group.location.end_line)
            .map(|(index, _)| index);

        let index = preceding.or_else(|| {
            self.groups.iter().position(|group| {
                !group.used
                    && group.location.start_line == node.start_line
                    && group.location.start >= node.end
            })
        })?;

        let group = &mut self.groups[index];
        group.used = true;
        Some(Docstring {
            text: SourceText::from_vec(group.text.clone()),
            location: group.location,
        })
    }

    /// Consumes the index and returns comment blocks that were not attached to
    /// any code object.
    pub(crate) fn into_unused(self) -> Vec<CommentEvent> {
        self.groups
            .into_iter()
            .filter(|group| !group.used)
            .map(|group| CommentEvent {
                text: SourceText::from_vec(group.text),
                location: group.location,
            })
            .collect()
    }
}

fn is_ignored(kind: CommentKind, raw: &[u8], line: u32) -> bool {
    if kind != CommentKind::Inline {
        return false;
    }

    let trimmed = trim_start_ascii(raw);
    if line == 1 && trimmed.starts_with(b"#!") {
        return true;
    }

    let without_hash = trimmed.strip_prefix(b"#").unwrap_or(trimmed);
    let body = trim_start_ascii(without_hash).to_ascii_lowercase();
    body.starts_with(b"frozen_string_literal:")
        || body.starts_with(b"encoding:")
        || body.starts_with(b"coding:")
        || body.starts_with(b"-*- coding")
        || body.starts_with(b"warn_indent:")
}

fn strip_inline(raw: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();

    for (index, line) in split_lines(raw).into_iter().enumerate() {
        if index > 0 {
            result.push(b'\n');
        }
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let line = line.strip_prefix(b"#").unwrap_or(line);
        let line = line.strip_prefix(b" ").unwrap_or(line);
        result.extend_from_slice(line);
    }

    result
}

fn strip_embdoc(raw: &[u8]) -> Vec<u8> {
    let lines = split_lines(raw);
    let mut start = 0;
    let mut end = lines.len();

    if lines
        .first()
        .is_some_and(|line| trim_start_ascii(line).starts_with(b"=begin"))
    {
        start = 1;
    }
    if end > start
        && lines
            .get(end - 1)
            .is_some_and(|line| trim_start_ascii(line).starts_with(b"=end"))
    {
        end -= 1;
    }

    join_lines(&lines[start..end])
}

fn split_lines(raw: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = raw.split(|byte| *byte == b'\n').collect();
    if raw.ends_with(b"\n") && lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

fn join_lines(lines: &[&[u8]]) -> Vec<u8> {
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            result.push(b'\n');
        }
        result.extend_from_slice(line);
    }
    result
}

fn trim_start_ascii(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if matches!(first, b' ' | b'\t' | b'\r' | b'\n') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}
