//! Extraction of the agent's ```factory-files block from a complete message.
use super::{MAX_ATTACHMENTS_PER_MESSAGE, MAX_REFERENCE_BYTES};

pub const FILES_BLOCK_INFO: &str = "factory-files";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedFiles {
    /// The message without its file blocks.
    pub text: String,
    /// Distinct references in order of appearance, at most one message's worth.
    pub references: Vec<String>,
    /// Lines that were too long, malformed or beyond the per-message limit.
    pub ignored: usize,
}

/// Fence opened by at most three spaces, then a run of backticks or tildes.
struct Fence<'a> {
    marker: char,
    length: usize,
    info: &'a str,
}

fn fence(line: &str) -> Option<Fence<'_>> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let marker = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let length = rest.chars().take_while(|c| *c == marker).count();
    let info = rest[length..].trim();
    (length >= 3 && !(marker == '`' && info.contains('`'))).then_some(Fence {
        marker,
        length,
        info,
    })
}

fn closes(line: &str, open: &Fence<'_>) -> bool {
    fence(line)
        .is_some_and(|f| f.marker == open.marker && f.length >= open.length && f.info.is_empty())
}

/// Remove every complete top-level `factory-files` block and collect its paths, one
/// per line. Blocks inside other fenced code and blocks that never close are left in
/// the text. Text without a block is returned unchanged.
pub fn extract_factory_files(text: &str) -> ExtractedFiles {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut found = ExtractedFiles::default();
    let mut removed = false;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let Some(open) = fence(line) else {
            kept.push(line);
            index += 1;
            continue;
        };
        let end = (index + 1..lines.len()).find(|i| closes(lines[*i], &open));
        let files_end = end.filter(|_| open.marker == '`' && open.info == FILES_BLOCK_INFO);
        if let Some(end) = files_end {
            collect(&lines[index + 1..end], &mut found);
            removed = true;
            index = end + 1;
            // Do not leave an empty gap where the block was.
            if kept.last().is_some_and(|l| l.trim().is_empty())
                && lines.get(index).is_some_and(|l| l.trim().is_empty())
            {
                index += 1;
            }
            continue;
        }
        // Other fenced code is opaque; an unclosed fence runs to the end.
        let stop = end.map_or(lines.len(), |end| end + 1);
        kept.extend(&lines[index..stop]);
        index = stop;
    }
    if !removed {
        found.text = text.to_owned();
        return found;
    }
    while kept.first().is_some_and(|l| l.trim().is_empty()) {
        kept.remove(0);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    found.text = kept.join("\n");
    found
}

fn collect(block: &[&str], found: &mut ExtractedFiles) {
    for line in block {
        let path = line.trim();
        if path.is_empty() || found.references.iter().any(|known| known == path) {
            continue;
        }
        if path.len() > MAX_REFERENCE_BYTES
            || path.chars().any(char::is_control)
            || found.references.len() >= MAX_ATTACHMENTS_PER_MESSAGE
        {
            found.ignored += 1;
        } else {
            found.references.push(path.to_owned());
        }
    }
}
