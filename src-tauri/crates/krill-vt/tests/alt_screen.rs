//! Alternate screen buffer behavior (modes 47/1047/1049).

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

#[test]
fn alt_screen_preserves_primary_content() {
    let p = run(b"primary\x1b[?1049h\x1b[HALT-SCREEN", 20, 5);
    assert!(p.screen.text().contains("ALT-SCREEN"));
    assert!(!p.screen.text().contains("primary"));
    assert!(p.screen.modes().alt_screen);

    let p = run(b"primary\x1b[?1049hALT\x1b[?1049l", 20, 5);
    assert!(p.screen.text().contains("primary"));
    assert!(!p.screen.modes().alt_screen);
}

#[test]
fn mode_1049_restores_cursor_position() {
    // Park the cursor mid-screen, enter alt (1049 saves it), draw, leave.
    let p = run(b"\x1b[2;5Hbefore\x1b[?1049h\x1b[HALT\x1b[?1049l", 20, 5);
    // Cursor restored to just after "before": col 5 + 6 chars = 10, row 2 (idx 1).
    assert_eq!(p.screen.cursor(), (10, 1));
    assert!(p.screen.text().contains("before"));
}

#[test]
fn bare_47_does_not_save_cursor() {
    let p = run(b"\x1b[3;3Hx\x1b[?47h\x1b[Halt\x1b[?47l", 20, 5);
    // Bare 47: cursor is homed by xterm conventions, not restored to (1,1).
    assert_eq!(p.screen.cursor(), (0, 0));
}

#[test]
fn alt_screen_is_fresh_and_cleared_on_exit() {
    // Entering twice leaves no residue of the previous alt content.
    let p = run(b"\x1b[?1049hold-alt\x1b[?1049l\x1b[?1049h", 20, 5);
    assert!(!p.screen.text().contains("old-alt"));

    // Content written on the alt screen never leaks into scrollback.
    let p = run(
        b"keep\r\nme\r\n\x1b[?1049h\n\n\n\n\n\n\n\n\x1b[?1049l",
        10,
        4,
    );
    assert!(p.screen.text().contains("keep"));
    assert!(p.screen.scrollback_len() <= 1);
}

#[test]
fn ris_resets_everything() {
    let p = run(b"\x1b[2;4rjunk\x1b[?1049h\x1bc", 10, 5);
    assert!(!p.screen.modes().alt_screen);
    assert_eq!(p.screen.cursor(), (0, 0));
}
