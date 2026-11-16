//! Splitting an `AGENTS.md` into rules by its structure only: one rule per paragraph or list item
//! under each `#` heading, each with the lines it came from. Nothing here reads what a sentence
//! means; the owner sets every rule's level in the draft.

/// One paragraph or list item of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub text: String,
    /// First and last line, counted from 1.
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Split {
    /// The first paragraph before any heading: what the app is.
    pub what: String,
    pub rules: Vec<Piece>,
}

struct Block {
    lines: Vec<String>,
    start: u32,
    end: u32,
    /// How many headings came before it (0: before the first).
    section: u32,
    /// A list item's indent; `None` for a paragraph.
    item: Option<usize>,
    /// It holds a fenced code block, whose lines keep their breaks.
    fenced: bool,
}

impl Block {
    fn text(&self) -> String {
        if self.fenced { self.lines.join("\n").trim().to_owned() } else { self.lines.join(" ").split_whitespace().collect::<Vec<_>>().join(" ") }
    }
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).map(|ch| if ch == '\t' { 4 } else { 1 }).sum()
}

/// The text of a list item without its marker (`- `, `* `, `+ `, `1. `, `1) `).
fn item_text(trimmed: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some(rest.trim_start());
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let rest = &trimmed[digits..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    rest.starts_with(' ').then(|| rest.trim_start())
}

fn heading(trimmed: &str) -> bool {
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    (1..=6).contains(&hashes) && trimmed[hashes..].chars().next().is_none_or(char::is_whitespace)
}

/// A line of only `-`, `*` or `_` (three or more): a break between parts, not text.
fn divider(trimmed: &str) -> bool {
    let marks: Vec<char> = trimmed.chars().filter(|ch| !ch.is_whitespace()).collect();
    marks.len() >= 3 && ['-', '*', '_'].contains(&marks[0]) && marks.iter().all(|ch| *ch == marks[0])
}

fn fence(trimmed: &str) -> Option<&'static str> {
    ["```", "~~~"].into_iter().find(|marker| trimmed.starts_with(marker))
}

pub fn split(text: &str) -> Split {
    let mut blocks: Vec<Block> = Vec::new();
    // Whether the last block still takes lines.
    let mut open = false;
    let mut section = 0u32;
    let mut in_fence: Option<&'static str> = None;
    for (index, line) in text.lines().enumerate() {
        let number = index as u32 + 1;
        let trimmed = line.trim();
        if let Some(marker) = in_fence {
            if let Some(block) = blocks.last_mut() {
                block.lines.push(line.trim_end().to_owned());
                block.end = number;
            }
            if trimmed.starts_with(marker) {
                in_fence = None;
            }
            continue;
        }
        if let Some(marker) = fence(trimmed) {
            // A code block belongs to the text above it under the same heading.
            let joins = blocks.last().is_some_and(|block| block.section == section);
            if !joins {
                blocks.push(Block { lines: Vec::new(), start: number, end: number, section, item: None, fenced: true });
            }
            let block = blocks.last_mut().expect("a block");
            block.fenced = true;
            block.lines.push(trimmed.to_owned());
            block.end = number;
            open = true;
            in_fence = Some(marker);
            continue;
        }
        if trimmed.is_empty() {
            open = false;
            continue;
        }
        if heading(trimmed) {
            section += 1;
            open = false;
            continue;
        }
        if divider(trimmed) || (trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
            open = false;
            continue;
        }
        if let Some(rest) = item_text(trimmed) {
            // An item indented under the item above it is part of that item.
            let nested = blocks.last().is_some_and(|block| block.section == section && block.item.is_some_and(|at| indent(line) > at));
            if nested {
                let block = blocks.last_mut().expect("a block");
                block.lines.push(rest.to_owned());
                block.end = number;
            } else {
                blocks.push(Block { lines: vec![rest.to_owned()], start: number, end: number, section, item: Some(indent(line)), fenced: false });
            }
            open = true;
            continue;
        }
        if open && let Some(block) = blocks.last_mut() {
            block.lines.push(trimmed.to_owned());
            block.end = number;
        } else {
            blocks.push(Block { lines: vec![trimmed.to_owned()], start: number, end: number, section, item: None, fenced: false });
            open = true;
        }
    }
    let mut out = Split::default();
    for block in blocks {
        let text = block.text();
        if text.is_empty() {
            continue;
        }
        if out.what.is_empty() && block.section == 0 && block.item.is_none() && !block.fenced {
            out.what = text;
        } else {
            out.rules.push(Piece { text, start: block.start, end: block.end });
        }
    }
    out
}
