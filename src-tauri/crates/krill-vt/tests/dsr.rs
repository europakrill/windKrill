use krill_core::Screen;
use krill_vt::TermPerformer;
use vte::Parser;

#[test]
fn dsr_cursor_position_query_queues_cpr_response() {
    let mut parser = Parser::new();
    let mut performer = TermPerformer::new(Screen::new(80, 24));

    for &byte in b"\x1b[3;5H\x1b[6n" {
        parser.advance(&mut performer, byte);
    }

    assert_eq!(performer.take_responses(), vec![b"\x1b[3;5R".to_vec()]);
    assert!(performer.take_responses().is_empty());
}

#[test]
fn dsr_clamps_wrap_pending_cursor_to_last_visible_column() {
    let mut parser = Parser::new();
    let mut performer = TermPerformer::new(Screen::new(4, 2));

    for &byte in b"abcd\x1b[6n" {
        parser.advance(&mut performer, byte);
    }

    assert_eq!(performer.take_responses(), vec![b"\x1b[1;4R".to_vec()]);
}
