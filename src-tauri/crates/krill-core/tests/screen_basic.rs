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
