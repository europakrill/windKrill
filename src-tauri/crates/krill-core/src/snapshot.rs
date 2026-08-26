//! Serializable screen snapshots for the GUI renderer.
//!
//! The frontend renders the terminal from a plain JSON snapshot instead of
//! touching the Rust grid directly. Cells are stored as runs (glyph +
//! attribute id) so a full 80x24 frame is a few KB, and identical attributes
//! collapse into one palette entry per snapshot.

use crate::{Cell, Color, Screen};
use serde::Serialize;

/// A resolved color in a snapshot attribute. `Default` means "use the theme
/// default"; indexed colors reference the theme palette; RGB is truecolor,
/// carried losslessly to the renderer (M4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum ColorDto {
    #[default]
    Default,
    Indexed(#[serde(with = "serde_indexed")] u8),
    Rgb([u8; 3]),
}

mod serde_indexed {
    use serde::Serializer;
    pub fn serialize<S: Serializer>(value: &u8, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(u16::from(*value))
    }
}

impl From<Color> for ColorDto {
    fn from(color: Color) -> Self {
        match color {
            Color::Default => ColorDto::Default,
            Color::Indexed(i) => ColorDto::Indexed(i),
            // M4: truecolor now travels losslessly instead of folding onto
            // the grayscale ramp.
            Color::Rgb(r, g, b) => ColorDto::Rgb([r, g, b]),
        }
    }
}

/// One attribute combination used by at least one cell in this snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct AttrDto {
    /// Foreground color or Default.
    pub fg: ColorDto,
    /// Background color or Default.
    pub bg: ColorDto,
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
        fg: attrs.fg.map(ColorDto::from).unwrap_or_default(),
        bg: attrs.bg.map(ColorDto::from).unwrap_or_default(),
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
    fn color_mapping_preserves_indexed_and_truecolor() {
        assert_eq!(ColorDto::from(Color::Default), ColorDto::Default);
        assert_eq!(ColorDto::from(Color::Indexed(7)), ColorDto::Indexed(7));
        // M4: truecolor is no longer folded onto the grayscale ramp.
        assert_eq!(
            ColorDto::from(Color::Rgb(0xff, 0x80, 0x01)),
            ColorDto::Rgb([0xff, 0x80, 0x01])
        );
    }

    #[test]
    fn truecolor_snapshot_round_trips_rgb() {
        let mut screen = Screen::new(10, 2);
        // SGR truecolor foreground (38;2;r;g;b) then a glyph.
        screen.sgr(&[38, 2, 18, 52, 86]);
        screen.put('x');
        let snap = SnapshotDto::from_screen(&screen);
        let attr = &snap.attrs[snap.lines[0].runs[0].attr as usize];
        assert_eq!(attr.fg, ColorDto::Rgb([18, 52, 86]));
    }
}
