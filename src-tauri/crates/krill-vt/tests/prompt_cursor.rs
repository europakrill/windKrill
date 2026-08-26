//! Snapshot cursor position must match the on-screen prompt caret.
//!
//! Regression for the M5 GUI bug: the rendered caret sat one row below the
//! prompt line at a stale column, because the frontend polled a snapshot that
//! was taken while the shell was mid-write (banner still streaming). The
//! snapshot itself is consistent — these tests pin the invariants.

use krill_core::Screen;
use krill_vt::{ParserConfig, TermPerformer, VtParser};

fn feed(screen: &mut Screen, data: &[u8]) {
    let cols = screen.cols();
    let rows = screen.rows();
    let mut p = VtParser::new(ParserConfig::default());
    let mut performer = TermPerformer::new(std::mem::replace(screen, Screen::new(cols, rows)));
    p.advance(data, &mut performer);
    performer.flush();
    *screen = performer.screen;
}

/// A bare LF moves down but keeps the column (no implicit CR).
#[test]
fn bare_lf_keeps_column() {
    let mut s = Screen::new(40, 4);
    feed(&mut s, b"abc\ndef");
    assert_eq!(s.cursor(), (6, 1));
    let text = s.text();
    let line2 = text.lines().nth(1).unwrap().trim_end();
    assert_eq!(line2, "   def");
}

/// Prompt sequence: banner CRLF, blank CRLF, then prompt text without a
/// trailing newline. Cursor parks right after "> ".
#[test]
fn powershell_prompt_cursor_parks_after_prompt() {
    let mut s = Screen::new(80, 24);
    feed(&mut s, b"PowerShell 7.5.0\r\n\r\nPS C:\\Users\\root> ");
    assert_eq!(s.cursor(), ("PS C:\\Users\\root> ".len() as u16, 2));
}

/// Writing to the last column leaves the wrap-pending sentinel internally,
/// but visible_cursor must clamp it back into the grid.
#[test]
fn write_to_last_col_visible_cursor_clamped() {
    let mut s = Screen::new(10, 2);
    feed(&mut s, b"0123456789");
    let (col, _row) = s.visible_cursor();
    assert!(col < s.cols());
}
