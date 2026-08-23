use krill_core::{Color, Modes, Screen};
use krill_vt::{ParserConfig, TermPerformer, VtParser};

/// Build a parser+performer pipeline and feed bytes.
fn feed(screen: &mut Screen, data: &[u8]) {
    let cols = screen.cols();
    let rows = screen.rows();
    let mut p = VtParser::new(ParserConfig::default());
    let mut performer = TermPerformer::new(std::mem::replace(screen, Screen::new(cols, rows)));
    p.advance(data, &mut performer);
    performer.flush();
    *screen = performer.screen;
}

#[test]
fn plain_text_passes_through() {
    let mut s = Screen::new(20, 4);
    feed(&mut s, b"hello");
    assert_eq!(s.cursor(), (5, 0));
    assert_eq!(s.text().lines().next().unwrap().len(), 20);
    assert!(s.text().lines().next().unwrap().starts_with("hello"));
}

#[test]
fn sgr_strips_and_applies_color() {
    let mut s = Screen::new(20, 4);
    feed(&mut s, b"\x1b[1;31mOK\x1b[0m");
    // 'O' must be bold red.
    let cell = s.cell(0, 0).unwrap();
    assert!(cell.attrs.bold);
    assert_eq!(cell.attrs.fg, Some(Color::Indexed(1)));
    // After reset, the next write would be default attrs.
    feed(&mut s, b"X");
    let cell = s.cell(0, 2).unwrap();
    assert!(!cell.attrs.bold);
    assert_eq!(cell.attrs.fg, None);
}

#[test]
fn truecolor_semicolon_form() {
    let mut s = Screen::new(20, 4);
    feed(&mut s, b"\x1b[38;2;12;34;56mZ");
    assert_eq!(s.cell(0, 0).unwrap().attrs.fg, Some(Color::Rgb(12, 34, 56)));
}

#[test]
fn truecolor_colon_form() {
    let mut s = Screen::new(20, 4);
    feed(&mut s, b"\x1b[38:2:9:8:7mZ");
    assert_eq!(s.cell(0, 0).unwrap().attrs.fg, Some(Color::Rgb(9, 8, 7)));
}

#[test]
fn cursor_positioning_cup() {
    let mut s = Screen::new(10, 10);
    feed(&mut s, b"\x1b[3;5HX");
    assert_eq!(s.cursor(), (5, 2));
    assert_eq!(s.cell(2, 4).unwrap().ch, 'X');
}

#[test]
fn cursor_movement() {
    let mut s = Screen::new(40, 40);
    feed(&mut s, b"\x1b[10;10H"); // (9, 9)
    feed(&mut s, b"\x1b[2A"); // up 2 -> row 7
    assert_eq!(s.cursor(), (9, 7));
    feed(&mut s, b"\x1b[3B"); // down 3 -> row 10
    assert_eq!(s.cursor(), (9, 10));
    feed(&mut s, b"\x1b[4D"); // left 4 -> col 5
    assert_eq!(s.cursor(), (5, 10));
    feed(&mut s, b"\x1b[2C"); // right 2 -> col 7
    assert_eq!(s.cursor(), (7, 10));
}

#[test]
fn erase_display_2_clears_all() {
    let mut s = Screen::new(10, 3);
    feed(&mut s, b"garbage\x1b[2J");
    assert!(s.text().chars().all(|c| c == ' ' || c == '\n'));
}

#[test]
fn private_modes_mouse_and_paste() {
    let mut s = Screen::new(10, 3);
    feed(&mut s, b"\x1b[?1000;1006;2004h");
    assert!(
        s.modes()
            == &Modes {
                mouse_normal: true,
                mouse_sgr: true,
                bracketed_paste: true,
                ..Default::default()
            }
    );
    feed(&mut s, b"\x1b[?2004l");
    assert!(!s.modes().bracketed_paste);
}

#[test]
fn scrollback_accumulates_on_scroll() {
    let mut s = Screen::with_scrollback_cap(4, 2, 100);
    // Write 6 lines worth of content into a 2-row screen.
    for line in ["aaaa", "bbbb", "cccc", "dddd", "eeee", "ffff"] {
        feed(&mut s, line.as_bytes());
        feed(&mut s, b"\r\n");
    }
    assert_eq!(s.scrollback_len(), 5); // lines a..e scrolled off
    assert_eq!(
        s.scrollback_line(0)
            .map(|r| r.iter().map(|c| c.ch).collect::<String>()),
        Some("aaaa".to_string())
    );
}

#[test]
fn insert_and_delete_chars() {
    use krill_vt::TermPerformer;
    let mut s = Screen::new(10, 1);
    let mut p = VtParser::new(ParserConfig::default());
    let mut pf = TermPerformer::new(s);
    p.advance(b"abcdef", &mut pf);
    // Move to col 0 and insert 2 blanks -> "  abcdef"
    p.advance(b"\x1b[H", &mut pf);
    p.advance(b"\x1b[2@", &mut pf);
    s = pf.screen;
    assert_eq!(s.text().lines().next().unwrap(), "  abcdef  ");
}
