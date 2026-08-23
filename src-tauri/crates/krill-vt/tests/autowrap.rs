//! DECAWM auto-wrap (mode 7) behavior.

use krill_core::Screen;
use krill_vt::TermPerformer;
use vte::Parser;

fn run(input: &[u8], cols: u16, rows: u16) -> TermPerformer {
    let mut parser = Parser::new();
    let mut performer = TermPerformer::new(Screen::new(cols, rows));
    for &b in input {
        parser.advance(&mut performer, b);
    }
    performer.flush();
    performer
}

fn rows_of(p: &TermPerformer) -> Vec<String> {
    p.screen
        .text()
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect()
}

#[test]
fn autowrap_on_wraps_to_next_line() {
    let p = run(b"ABCDEFGHIJKLM", 10, 3);
    let lines = rows_of(&p);
    assert_eq!(lines[0], "ABCDEFGHIJ");
    assert_eq!(lines[1], "KLM");
}

#[test]
fn autowrap_off_overwrites_last_column() {
    // Width 10: J lands in the last column, then K/L/M keep overwriting it.
    let p = run(b"\x1b[?7lABCDEFGHIJKLM", 10, 3);
    let lines = rows_of(&p);
    assert_eq!(lines[0], "ABCDEFGHIM");
    assert!(lines[1].is_empty());
}

#[test]
fn deferred_wrap_pending_flag_semantics() {
    // After filling the row exactly, the wrap is deferred until the next
    // printable char. A CR+LF rewrite must not produce a stray blank line.
    let p = run(b"0123456789\r\nX", 10, 3);
    let lines = rows_of(&p);
    assert_eq!(lines[0], "0123456789");
    assert_eq!(lines[1], "X");
}
