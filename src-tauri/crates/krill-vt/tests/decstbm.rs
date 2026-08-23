//! DECSTBM scroll region behavior (CSI r, SU/SD, RI inside a region).

use krill_core::Screen;
use krill_vt::TermPerformer;
use vte::Parser;

fn screen_after(input: &[u8], cols: u16, rows: u16) -> String {
    let mut parser = Parser::new();
    let mut performer = TermPerformer::new(Screen::new(cols, rows));
    for &b in input {
        parser.advance(&mut performer, b);
    }
    performer.flush();
    performer.screen.text()
}

#[test]
fn decstbm_region_scrolls_without_touching_outside_lines() {
    // 5 rows; fill each with its number.
    let feed = b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r";
    let text = screen_after(feed, 10, 5);
    assert!(text.starts_with("1"));

    // LF at the bottom margin (row 4) must scroll only rows 2..=4.
    let text = screen_after(
        b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[2;1HX\x1b[4;1HY\r\n",
        10,
        5,
    );
    // Row 1 untouched ("1"), row 5 untouched ("5").
    assert!(text.starts_with("1"));
    assert!(text.lines().last().unwrap_or("").starts_with('5'));
    // Inside: X scrolled up out, Y moved to top of region (row 2).
    assert!(!text.contains('X'));
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    assert_eq!(lines[2], "Y");
}

#[test]
fn decstbm_produces_no_scrollback() {
    let mut parser = Parser::new();
    let mut p = TermPerformer::new(Screen::new(10, 5));
    for &b in b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r" {
        parser.advance(&mut p, b);
    }
    for _ in 0..10 {
        parser.advance(&mut p, b'\n');
    }
    p.flush();
    assert_eq!(
        p.screen.scrollback_len(),
        0,
        "region scrolling must not push into scrollback"
    );
}

#[test]
fn decstbm_ignores_invalid_region_and_homes_cursor_on_valid_set() {
    let mut parser = Parser::new();
    let mut p = TermPerformer::new(Screen::new(10, 5));
    // Invalid: top >= bottom -> ignored.
    for &b in b"\x1b[4;3r" {
        parser.advance(&mut p, b);
    }
    // Valid set homes the cursor per spec.
    for &b in b"\x1b[2;4r" {
        parser.advance(&mut p, b);
    }
    p.flush();
    assert_eq!(p.screen.cursor(), (0, 0));
}

#[test]
fn su_scrolls_region_contents_cursor_stays_put() {
    // Rows: A B C D E. Region 2..=4. CSI 1 S scrolls rows 2..=4 up once.
    let text = screen_after(b"A\r\nB\r\nC\r\nD\r\nE\x1b[2;4r\x1b[1S", 10, 5);
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    assert_eq!(lines[0], "A");
    assert_eq!(lines[1], "C"); // was row 3
    assert_eq!(lines[2], "D"); // was row 4
    assert!(lines[3].trim().is_empty()); // blank filled
    assert_eq!(lines[4], "E");
}

#[test]
fn reverse_index_scrolls_down_at_region_top() {
    let text = screen_after(b"A\r\nB\r\nC\r\nD\r\nE\x1b[2;4r\x1b[2;1HZ\x1bM", 10, 5);
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    assert_eq!(lines[0], "A");
    // Z at region top scrolled down: row 2 becomes blank, Z lands on row 3.
    assert!(lines[1].is_empty());
    assert_eq!(lines[2], "Z");
    assert_eq!(lines[4], "E");
}
