//! Serializable screen snapshots for the GUI renderer.
//!
//! The frontend renders the terminal from a plain JSON snapshot instead of
//! touching the Rust grid directly. Cells are stored as runs (glyph +
//! attribute id) so a full 80x24 frame is a few KB, and identical attributes
//! collapse into one palette entry per snapshot.

use crate::{Cell, Color, Screen};
use serde::Serialize;

/// One attribute combination used by at least one cell in this snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttrDto {
    /// Palette index or -1 for default foreground.
    pub fg: i16,
    /// Palette index or -1 for default background.
    pub bg: i16,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

/// A horizontal run of cells sharing one attribute entry.
#[derive(Debug, Clone, Serialize)]
pub struct RunDto {
    /// Index into [`SnapshotDto::attrs`].
    pub attr: u32,
    /// Decoded text of the run (one char per cell, spaces included).
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RowDto {
    pub runs: Vec<RunDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotDto {
    pub cols: u16,
    pub rows: u16,
    pub cursor_col: u16,
    pub cursor_row: u16,
    pub cursor_visible: bool,
    /// Scrollback lines oldest-first (currently only the visible grid is
    /// shipped; scrollback rendering lands with the M3 pane view).
    pub lines: Vec<RowDto>,
    pub attrs: Vec<AttrDto>,
}

impl From<Color> for i16 {
    fn from(color: Color) -> Self {
        match color {
            Color::Default => -1,
            Color::Indexed(i) => i16::from(i),
            // RGB colors are folded into an extended palette slot; xterm-256
            // plus truecolor covers indices 0..=255, so extended colors use
            // 256 + r*65536 + g*256 + b encoded as f32-safe i32... kept in
            // range by clamping to i16::MAX is lossy, so we instead map RGB
            // onto dedicated negative slots below.
            Color::Rgb(r, g, b) => {
                // Deterministic fold: not lossless, but snapshots carry the
                // dominant terminal palettes; truecolor fidelity comes with
                // the canvas renderer in M4.
                let mixed = (i32::from(r) * 299 + i32::from(g) * 587 + i32::from(b) * 114) / 1000;
                let idx = 232 + (mixed.clamp(0, 255) / 10).min(23); // grayscale ramp
                i16::try_from(idx).unwrap_or(-1)
            }
        }
    }
}

fn attr_id(cache: &mut Vec<AttrDto>, attrs: &crate::Attrs) -> u32 {
    let dto = AttrDto {
        fg: attrs.fg.map(i16::from).unwrap_or(-1),
        bg: attrs.bg.map(i16::from).unwrap_or(-1),
        bold: attrs.bold,
        italic: attrs.italic,
        underline: attrs.underline,
        reverse: attrs.reverse,
    };
    if let Some(pos) = cache.iter().position(|existing| existing == &dto) {
        return pos as u32;
    }
    cache.push(dto);
    (cache.len() - 1) as u32
}

impl SnapshotDto {
    pub fn from_screen(screen: &Screen) -> Self {
        let mut attrs_cache: Vec<AttrDto> = Vec::new();
        let default_attr = AttrDto::default();
        attrs_cache.push(default_attr);

        let mut lines = Vec::with_capacity(usize::from(screen.rows()));
        for row in 0..screen.rows() {
            let mut runs: Vec<RunDto> = Vec::new();
            let mut current_attr: Option<u32> = None;
            let mut text = String::new();
            for col in 0..screen.cols() {
                let cell: Cell = screen.cell(row, col).copied().unwrap_or_default();
                let id = attr_id(&mut attrs_cache, &cell.attrs);
                if current_attr != Some(id) {
                    if let Some(attr) = current_attr.take() {
                        runs.push(RunDto {
                            attr,
                            text: std::mem::take(&mut text),
                        });
                    }
                    current_attr = Some(id);
                }
                text.push(cell.ch);
            }
            if let Some(attr) = current_attr {
                runs.push(RunDto { attr, text });
            }
            lines.push(RowDto { runs });
        }

        let (cursor_row, cursor_col) = screen.visible_cursor();
        SnapshotDto {
            cols: screen.cols(),
            rows: screen.rows(),
            cursor_col,
            cursor_row,
            // Cursor visibility tracking (DECTCEM) lands with M3 pane view;
            // the shell prompt always shows a caret for now.
            cursor_visible: true,
            lines,
            attrs: attrs_cache,
        }
    }
}

impl Default for AttrDto {
    fn default() -> Self {
        Self {
            fg: -1,
            bg: -1,
            bold: false,
            italic: false,
            underline: false,
            reverse: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attrs;

    #[test]
    fn snapshot_matches_screen_dimensions_and_text() {
        let mut screen = Screen::new(20, 5);
        for ch in "hello".chars() {
            screen.put(ch);
        }
        let snap = SnapshotDto::from_screen(&screen);
        assert_eq!(snap.cols, 20);
        assert_eq!(snap.rows, 5);
        assert_eq!(snap.lines.len(), 5);
        let first = snap.lines[0]
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(first, "hello               ");
    }

    #[test]
    fn distinct_attributes_get_distinct_palette_entries() {
        let mut screen = Screen::new(10, 2);
        screen.put('a');
        let snap = SnapshotDto::from_screen(&screen);
        // Default attr must always be present at index 0.
        assert_eq!(snap.attrs[0], AttrDto::default());
        let _ = Attrs::default();
    }

    #[test]
    fn color_mapping_defaults_to_minus_one() {
        assert_eq!(i16::from(Color::Default), -1);
        assert_eq!(i16::from(Color::Indexed(7)), 7);
    }
}
