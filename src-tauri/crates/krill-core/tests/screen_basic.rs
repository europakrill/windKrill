use krill_core::Screen;

#[test]
fn put_prints_and_advances_cursor() {
    let mut s = Screen::new(10, 3);
    for ch in "hello".chars() {
        s.put(ch);
    }
    assert_eq!(s.cursor(), (5, 0));
    assert_eq!(s.text().lines().next().unwrap(), "hello     ");
}

#[test]
fn wrap_at_right_margin() {
    let mut s = Screen::new(4, 3);
    for ch in "abcdef".chars() {
        s.put(ch);
    }
    assert_eq!(s.cursor(), (2, 1));
}

#[test]
fn scroll_keeps_last_rows() {
    let mut s = Screen::new(3, 2);
    for ch in "123456789".chars() {
        s.put(ch);
        if ch != '9' && s.cursor().0 == 3 {
            s.carriage_return();
            s.line_feed();
        } else if ch == '9' {
            break;
        }
    }
    // After scrolling, "123" has left the screen; rows are "456" / "789".
    let text = s.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.first(), Some(&"456"));
    assert_eq!(lines.get(1), Some(&"789"));
}

#[test]
fn resize_preserves_overlap_and_clamps_cursor() {
    let mut s = Screen::new(4, 2);
    for ch in "abcdef".chars() {
        s.put(ch);
    }

    s.resize(3, 3).unwrap();
    assert_eq!((s.cols(), s.rows()), (3, 3));
    let text = s.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "abc");
    assert_eq!(lines[1], "ef ");
    assert_eq!(s.cursor(), (2, 1));

    s.resize(2, 1).unwrap();
    assert_eq!(s.cursor(), (1, 0));
    assert_eq!(s.text(), "ab");
}

#[test]
fn resize_clears_wrap_pending_without_moving_right_when_widening() {
    let mut s = Screen::new(4, 2);
    for ch in "abcd".chars() {
        s.put(ch);
    }
    assert_eq!(s.cursor(), (4, 0), "precondition: wrap is pending");
    s.save_cursor();

    s.resize(8, 2).unwrap();
    assert_eq!(s.cursor(), (3, 0));
    s.goto(1, 1);
    s.restore_cursor();
    assert_eq!(s.cursor(), (3, 0));
}

#[test]
fn invalid_or_excessive_screen_sizes_are_rejected_before_allocation() {
    assert!(Screen::try_new(0, 24).is_err());
    assert!(Screen::try_new(80, 0).is_err());
    assert!(Screen::try_new(4096, 4096).is_err());

    let mut s = Screen::new(80, 24);
    assert!(s.resize(0, 24).is_err());
    assert!(s.resize(4096, 4096).is_err());
    assert_eq!((s.cols(), s.rows()), (80, 24));
}
