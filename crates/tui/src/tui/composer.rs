//! Composer input rendering, slash-command panel, `@`-mention completion, and
//! the categorised command palette.
//!
//! Extracted from `tui/app.rs` in Phase 2.2. Everything here operates on raw
//! buffer strings + character/byte offsets; no ratatui `Frame` / `Terminal`
//! knowledge, which keeps these helpers trivially unit-testable.

use crate::file_mentions;
use crate::slash_commands::{CommandCategory, visible_commands};
use crate::tui::app::TuiCmd;
use crate::tui::state::TuiSessionState;
use crate::tui::theme;
use crate::tui::transcript::wrap_text;
use nca_core::skills::{SkillCatalog, SkillSource};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::path::{Path, PathBuf};

pub const SLASH_PANEL_MAX_ROWS: usize = 8;

/// Text used for `/command` detection (ignores leading spaces in the composer).
pub fn slash_command_buffer(buffer: &str) -> &str {
    buffer.trim_start()
}

pub fn slash_panel_visible(buffer: &str) -> bool {
    let s = slash_command_buffer(buffer);
    s.starts_with('/') && !s.contains(' ')
}

pub fn cursor_byte_index(line: &str, cursor_char_idx: usize) -> usize {
    line.char_indices()
        .nth(cursor_char_idx)
        .map(|(i, _)| i)
        .unwrap_or(line.len())
}

pub fn at_panel_height(n: usize) -> u16 {
    if n == 0 {
        return 0;
    }
    (n.min(SLASH_PANEL_MAX_ROWS) as u16).saturating_add(2)
}

pub fn at_completion_active(buffer: &str, cursor_char_idx: usize) -> bool {
    if slash_panel_visible(buffer) {
        return false;
    }
    let b = cursor_byte_index(buffer, cursor_char_idx);
    file_mentions::at_token_before_cursor(buffer, b).is_some()
}

pub fn at_completion_matches(
    workspace_files: &[String],
    buffer: &str,
    cursor_char_idx: usize,
) -> Vec<String> {
    if !at_completion_active(buffer, cursor_char_idx) {
        return Vec::new();
    }
    let b = cursor_byte_index(buffer, cursor_char_idx);
    let Some((_, prefix)) = file_mentions::at_token_before_cursor(buffer, b) else {
        return Vec::new();
    };
    file_mentions::filter_paths_prefix(workspace_files, &prefix)
}

pub fn composer_chrome_height(
    slash_entries: &[SlashEntry],
    workspace_files: &[String],
    buffer: &str,
    cursor_char_idx: usize,
) -> u16 {
    let slash_filtered = filter_slash_entries(slash_entries, buffer);
    let at_matches = at_completion_matches(workspace_files, buffer, cursor_char_idx);
    let slash_h = if slash_panel_visible(buffer) {
        slash_panel_height(slash_filtered.len())
    } else {
        0
    };
    let at_h = if !at_matches.is_empty() {
        at_panel_height(at_matches.len())
    } else {
        0
    };
    slash_h.max(at_h)
}

/// Total height (borders + content) the input box needs: enough rows for the
/// word-wrapped input line, plus the staged-image note and/or status hint
/// when those are shown below it. Without this, a long line wraps visually
/// (ratatui's own `Wrap`) but the extra rows have no space reserved and get
/// clipped by the fixed-height layout.
pub fn composer_box_height(g: &TuiSessionState, width: u16) -> u16 {
    // 2 for the box borders, 2 for the "❯ " prompt prefix on the content line.
    let inner_w = width.saturating_sub(4).max(1) as usize;
    let content_rows = wrap_text(&g.input_buffer, inner_w).len().max(1) as u16;
    let hint_visible = g.active_approval.is_some()
        || (g.active_question.is_some() && !g.question_modal_open())
        || slash_panel_visible(&g.input_buffer)
        || g.input_buffer.is_empty();
    let staged_row: u16 = if g.staged_image_attachments.is_empty() {
        0
    } else {
        1
    };
    let hint_row: u16 = if hint_visible { 1 } else { 0 };
    2 + content_rows + staged_row + hint_row
}

/// Replace `@prefix` before cursor with `@choice` (relative path).
pub fn apply_at_completion(buffer: &str, cursor_char_idx: usize, choice: &str) -> (String, usize) {
    let b = cursor_byte_index(buffer, cursor_char_idx);
    let Some((at_byte, _prefix)) = file_mentions::at_token_before_cursor(buffer, b) else {
        return (buffer.to_string(), cursor_char_idx);
    };
    let before = &buffer[..at_byte.saturating_add(1)];
    let after = &buffer[b..];
    let new_buf = format!("{before}{choice}{after}");
    let new_byte = at_byte + 1 + choice.len();
    let new_char = new_buf[..new_byte.min(new_buf.len())].chars().count();
    (new_buf, new_char)
}

pub fn apply_selected_at_completion(
    workspace_files: &[String],
    buffer: &str,
    cursor_char_idx: usize,
    at_menu_index: usize,
    append_space: bool,
) -> Option<(String, usize)> {
    let at_matches = at_completion_matches(workspace_files, buffer, cursor_char_idx);
    if at_matches.is_empty() || !at_completion_active(buffer, cursor_char_idx) {
        return None;
    }

    let pick = at_menu_index.min(at_matches.len().saturating_sub(1));
    let choice = at_matches.get(pick)?;
    let (mut new_buf, mut new_cursor_char_idx) =
        apply_at_completion(buffer, cursor_char_idx, choice);

    if append_space {
        let insert_at = cursor_byte_index(&new_buf, new_cursor_char_idx);
        new_buf.insert(insert_at, ' ');
        new_cursor_char_idx += 1;
    }

    Some((new_buf, new_cursor_char_idx))
}

pub fn at_mention_char_ranges(buffer: &str) -> Vec<(usize, usize)> {
    file_mentions::parse_at_mentions(buffer)
        .into_iter()
        .map(|(start, end, _)| {
            let start_char = buffer[..start].chars().count();
            let end_char = buffer[..end].chars().count();
            (start_char, end_char)
        })
        .collect()
}

pub fn completed_at_mention_range_before_cursor(
    buffer: &str,
    cursor_char_idx: usize,
) -> Option<(usize, usize)> {
    let chars: Vec<char> = buffer.chars().collect();
    for (start_char, end_char) in at_mention_char_ranges(buffer) {
        if end_char == cursor_char_idx {
            return Some((start_char, end_char));
        }
        if end_char < chars.len()
            && end_char + 1 == cursor_char_idx
            && chars.get(end_char) == Some(&' ')
        {
            return Some((start_char, end_char + 1));
        }
    }
    None
}

pub fn remove_char_range(buffer: &str, start_char_idx: usize, end_char_idx: usize) -> String {
    let mut chars: Vec<char> = buffer.chars().collect();
    chars.drain(start_char_idx..end_char_idx);
    chars.into_iter().collect()
}

pub fn delete_completed_at_mention(
    buffer: &str,
    cursor_char_idx: usize,
) -> Option<(String, usize)> {
    let (start_char, end_char) = completed_at_mention_range_before_cursor(buffer, cursor_char_idx)?;
    Some((remove_char_range(buffer, start_char, end_char), start_char))
}

/// Ctrl+W (readline convention): delete back through the word behind the
/// cursor, skipping any trailing whitespace first. Returns the new buffer
/// and cursor position; `None` if the cursor is already at column 0.
pub fn delete_word_backward(buffer: &str, cursor_char_idx: usize) -> Option<(String, usize)> {
    if cursor_char_idx == 0 {
        return None;
    }
    let chars: Vec<char> = buffer.chars().collect();
    let mut start = cursor_char_idx.min(chars.len());
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    Some((remove_char_range(buffer, start, cursor_char_idx), start))
}

/// Ctrl+U (readline convention): delete from the cursor back to the start of
/// the current line — stops at the nearest preceding `\n` rather than the
/// start of the whole (possibly multi-line) buffer. `None` if the cursor is
/// already at the start of its line.
pub fn delete_to_line_start(buffer: &str, cursor_char_idx: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = buffer.chars().collect();
    let cursor_char_idx = cursor_char_idx.min(chars.len());
    let line_start = chars[..cursor_char_idx]
        .iter()
        .rposition(|&c| c == '\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    if line_start == cursor_char_idx {
        return None;
    }
    Some((remove_char_range(buffer, line_start, cursor_char_idx), line_start))
}

fn push_styled_run(
    spans: &mut Vec<Span<'static>>,
    text: &mut String,
    current_style: &mut Option<Style>,
    style: Style,
    ch: char,
) {
    if current_style.as_ref() != Some(&style) && !text.is_empty() {
        spans.push(Span::styled(
            std::mem::take(text),
            current_style.unwrap_or_default(),
        ));
    }
    *current_style = Some(style);
    text.push(ch);
}

/// Render the composer buffer as one `Line` per `\n`-separated row (Shift+Enter
/// / Ctrl+J insert a literal `\n` into the buffer for a multi-line prompt).
/// Only the first row gets the "❯ " prompt glyph; continuation rows are
/// indented to align under it.
pub fn composer_line(buffer: &str, cursor_char_idx: usize) -> Vec<Line<'static>> {
    let chars: Vec<char> = buffer.chars().collect();
    let mention_ranges = at_mention_char_ranges(buffer);
    let cursor_char_idx = cursor_char_idx.min(chars.len());
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans = vec![Span::styled("❯ ", Style::default().fg(theme::USER).bold())];
    let mut run = String::new();
    let mut run_style: Option<Style> = None;

    let cursor_style_for = |idx: usize| {
        let in_mention = idx < chars.len()
            && mention_ranges
                .iter()
                .any(|(start, end)| *start <= idx && idx < *end);
        if in_mention {
            Style::default()
                .bg(theme::USER)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .bg(theme::MUTED)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        }
    };

    for idx in 0..=chars.len() {
        if idx == cursor_char_idx {
            let cursor_char = chars.get(idx).copied().unwrap_or(' ');
            // Render the cursor as a highlighted block even when it sits on
            // the newline itself, then let the newline handling below (or
            // end-of-buffer) close the line out.
            let shown = if cursor_char == '\n' { ' ' } else { cursor_char };
            push_styled_run(&mut spans, &mut run, &mut run_style, cursor_style_for(idx), shown);
            if cursor_char == '\n' {
                if !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), run_style.take().unwrap_or_default()));
                }
                out.push(Line::from(std::mem::take(&mut spans)));
                spans = vec![Span::raw("  ")];
                continue;
            }
            if idx == chars.len() {
                break;
            }
            continue;
        }

        let Some(ch) = chars.get(idx).copied() else {
            break;
        };
        if ch == '\n' {
            if !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), run_style.take().unwrap_or_default()));
            }
            out.push(Line::from(std::mem::take(&mut spans)));
            spans = vec![Span::raw("  ")];
            continue;
        }
        let in_mention = mention_ranges
            .iter()
            .any(|(start, end)| *start <= idx && idx < *end);
        let style = if in_mention {
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::MENTION_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT)
        };
        push_styled_run(&mut spans, &mut run, &mut run_style, style, ch);
    }

    if !run.is_empty() {
        spans.push(Span::styled(run, run_style.unwrap_or_default()));
    }
    out.push(Line::from(spans));
    out
}

// ---------------------------------------------------------------------------
// Slash-command entries
// ---------------------------------------------------------------------------

/// Entry for the slash panel: either a hardcoded command or a discovered skill.
#[derive(Clone)]
pub enum SlashEntry {
    Command(&'static str),
    Skill {
        command: String,
        description: Option<String>,
        source: SkillSource,
    },
}

impl SlashEntry {
    pub fn command_str(&self) -> String {
        match self {
            SlashEntry::Command(s) => s.to_string(),
            SlashEntry::Skill { command, .. } => format!("/{command}"),
        }
    }

    pub fn display_text(&self) -> String {
        match self {
            SlashEntry::Command(s) => s.to_string(),
            SlashEntry::Skill {
                command,
                description,
                source,
            } => {
                let tag = match source {
                    SkillSource::AgentsMd => " (AGENTS.md)",
                    SkillSource::FileSystem => " (skill dir)",
                };
                match description {
                    Some(desc) => format!("/{command:<20} — {desc}{tag}"),
                    None => format!("/{command}{tag}"),
                }
            }
        }
    }
}

/// Collect skills from `SkillCatalog` for slash panel display.
fn collect_skill_entries(workspace_root: &Path, skill_dirs: &[PathBuf]) -> Vec<SlashEntry> {
    match SkillCatalog::discover(workspace_root, skill_dirs) {
        Ok(skills) => skills
            .into_iter()
            .map(|s| SlashEntry::Skill {
                command: s.command,
                description: s.description,
                source: s.source,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Load all slash-commands: hardcoded commands + discovered skills.
pub fn load_slash_entries(workspace_root: &Path, skill_dirs: &[PathBuf]) -> Vec<SlashEntry> {
    let mut entries: Vec<SlashEntry> = visible_commands()
        .map(|spec| SlashEntry::Command(spec.name))
        .collect();

    entries.extend(collect_skill_entries(workspace_root, skill_dirs));

    entries.sort_by(|a, b| {
        a.command_str()
            .to_lowercase()
            .cmp(&b.command_str().to_lowercase())
    });
    entries.dedup_by(|a, b| a.command_str().eq_ignore_ascii_case(&b.command_str()));
    entries
}

/// Filter slash entries by buffer prefix.
pub fn filter_slash_entries<'a>(entries: &'a [SlashEntry], buffer: &str) -> Vec<&'a SlashEntry> {
    if !slash_panel_visible(buffer) {
        return Vec::new();
    }
    let s = slash_command_buffer(buffer);
    let needle = s.trim_start_matches('/').to_lowercase();
    entries
        .iter()
        .filter(|e| {
            e.command_str()
                .trim_start_matches('/')
                .to_lowercase()
                .starts_with(&needle)
        })
        .collect()
}

pub fn slash_panel_height(filtered_len: usize) -> u16 {
    if filtered_len == 0 {
        return 0;
    }
    let rows = filtered_len.min(SLASH_PANEL_MAX_ROWS);
    let footer = if filtered_len > SLASH_PANEL_MAX_ROWS {
        1
    } else {
        0
    };
    (rows as u16)
        .saturating_add(footer)
        .saturating_add(2)
        .min(14)
}

// ---------------------------------------------------------------------------
// Branch picker
// ---------------------------------------------------------------------------

pub fn branch_filter_text(query: &str) -> &str {
    query.trim().strip_prefix('/').unwrap_or(query.trim())
}

pub fn filtered_branch_indices(branches: &[String], query: &str) -> Vec<usize> {
    let filter = branch_filter_text(query).to_ascii_lowercase();
    if filter.is_empty() {
        return (0..branches.len()).collect();
    }
    branches
        .iter()
        .enumerate()
        .filter(|(_, branch)| branch.to_ascii_lowercase().contains(&filter))
        .map(|(idx, _)| idx)
        .collect()
}

pub fn branch_picker_enter_command(
    branches: &[String],
    query: &str,
    selected_filtered_idx: usize,
) -> Option<TuiCmd> {
    let raw_query = query.trim();
    let branch_name = branch_filter_text(raw_query).trim();
    let filtered = filtered_branch_indices(branches, raw_query);

    if raw_query.starts_with('/') {
        return (!branch_name.is_empty()).then(|| TuiCmd::CreateBranch(branch_name.to_string()));
    }

    if !branch_name.is_empty()
        && let Some((idx, _)) = branches
            .iter()
            .enumerate()
            .find(|(_, branch)| branch.eq_ignore_ascii_case(branch_name))
    {
        return Some(TuiCmd::SwitchBranch(branches[idx].clone()));
    }

    filtered
        .get(selected_filtered_idx)
        .copied()
        .map(|idx| TuiCmd::SwitchBranch(branches[idx].clone()))
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

/// A row in the categorised command palette.
#[derive(Clone)]
pub enum PaletteRow {
    Section(&'static str),
    Entry {
        command: &'static str,
        label: &'static str,
        shortcut: &'static str,
    },
}

static PALETTE_CATALOG: std::sync::LazyLock<Vec<PaletteRow>> = std::sync::LazyLock::new(|| {
    let mut rows = Vec::new();
    for category in CommandCategory::ALL {
        rows.push(PaletteRow::Section(category.label()));
        rows.extend(
            visible_commands()
                .filter(|spec| spec.category == category)
                .map(|spec| PaletteRow::Entry {
                    command: spec.name,
                    label: spec.description,
                    shortcut: spec.shortcut,
                }),
        );
    }
    rows
});

pub fn palette_command_for_label(label: &str) -> &'static str {
    visible_commands()
        .find(|spec| spec.description == label)
        .map(|spec| spec.name)
        .unwrap_or("/help")
}

pub fn filter_palette_rows(query: &str) -> Vec<&'static PaletteRow> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return PALETTE_CATALOG.iter().collect();
    }
    let mut result: Vec<&'static PaletteRow> = Vec::new();
    let mut pending_section: Option<&'static PaletteRow> = None;
    for row in PALETTE_CATALOG.iter() {
        match row {
            PaletteRow::Section(_) => {
                pending_section = Some(row);
            }
            PaletteRow::Entry {
                command,
                label,
                shortcut,
            } => {
                if label.to_ascii_lowercase().contains(&needle)
                    || shortcut.to_ascii_lowercase().contains(&needle)
                    || command.contains(&needle)
                {
                    if let Some(s) = pending_section.take() {
                        result.push(s);
                    }
                    result.push(row);
                }
            }
        }
    }
    result
}

pub fn palette_selectable_indices(rows: &[&PaletteRow]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(i, r)| matches!(r, PaletteRow::Entry { .. }).then_some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slash_commands::resolve_command;

    #[test]
    fn delete_word_backward_skips_trailing_whitespace_then_the_word() {
        let (buf, cidx) = delete_word_backward("hello world  ", 13).unwrap();
        assert_eq!(buf, "hello ");
        assert_eq!(cidx, 6);
    }

    #[test]
    fn delete_word_backward_from_word_middle_deletes_only_that_word() {
        // Cursor right after "bar" (index 7): only "bar" is consumed, not the
        // space that follows it before "baz" — matches readline's Ctrl+W.
        let (buf, cidx) = delete_word_backward("foo bar baz", 7).unwrap();
        assert_eq!(buf, "foo  baz");
        assert_eq!(cidx, 4);
    }

    #[test]
    fn delete_word_backward_at_column_zero_is_none() {
        assert_eq!(delete_word_backward("hello", 0), None);
    }

    #[test]
    fn composer_line_splits_on_embedded_newlines() {
        let lines = composer_line("first\nsecond", 12);
        assert_eq!(lines.len(), 2);
        let first_plain: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let second_plain: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first_plain.contains("first"));
        assert!(second_plain.contains("second"));
    }

    #[test]
    fn visible_palette_entries_map_to_registered_commands() {
        for row in PALETTE_CATALOG.iter() {
            if let PaletteRow::Entry { command, .. } = row {
                assert!(
                    resolve_command(command).is_some(),
                    "{command} is not registered"
                );
            }
        }
    }

    #[test]
    fn connect_provider_is_not_duplicated() {
        let count = PALETTE_CATALOG
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    PaletteRow::Entry {
                        command: "/connect",
                        ..
                    }
                )
            })
            .count();
        assert_eq!(count, 1);
    }
}
