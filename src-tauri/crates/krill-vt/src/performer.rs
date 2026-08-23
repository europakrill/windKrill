//! `vte::Perform` bridge: translates parser events into `Screen` mutations.
//!
//! Uses vte 0.13's `Params` (semicolon-separated, each with optional
//! colon sub-params) so both `38;2;r;g;b` and `38:2:r:g:b` work.

use krill_core::Screen;
use vte::{Params, Perform};

/// Performer implementation driving a [`Screen`].
pub struct TermPerformer {
    pub screen: Screen,
    /// Pending printable chars, flushed before any control action.
    printed: String,
}

impl TermPerformer {
    pub fn new(screen: Screen) -> Self {
        Self {
            screen,
            printed: String::new(),
        }
    }

    fn flush_printed(&mut self) {
        for ch in self.printed.drain(..) {
            self.screen.put(ch);
        }
    }

    /// Flush pending printable chars into the screen. Call at frame /
    /// read-buffer boundaries so trailing text without a trailing control
    /// character still lands.
    pub fn flush(&mut self) {
        self.flush_printed();
    }

    fn handle_csi(&mut self, params: &Params, intermediates: &[u8], action: char) {
        self.flush_printed();
        let private = intermediates.first().copied();
        let first = |default: i64| -> i64 {
            params
                .iter()
                .next()
                .and_then(|p| p.first().copied())
                .map(i64::from)
                .unwrap_or(default)
        };

        match action {
            'A' => self.screen.move_cursor(-first(1).max(1), 0),
            'B' | 'e' => self.screen.move_cursor(first(1).max(1), 0),
            'C' | 'a' => self.screen.move_cursor(0, first(1).max(1)),
            'D' => self.screen.move_cursor(0, -first(1).max(1)),
            'E' => {
                self.screen.carriage_return();
                self.screen.move_cursor(first(1).max(1), 0);
            }
            'F' => {
                self.screen.carriage_return();
                self.screen.move_cursor(-first(1).max(1), 0);
            }
            'G' | '`' => {
                let col = first(1);
                self.screen.goto(i64::from(self.screen.cursor().1), col - 1);
            }
            'H' | 'f' => {
                let mut it = params.iter();
                let row = it.next().and_then(|p| p.first().copied()).unwrap_or(1);
                let col = it.next().and_then(|p| p.first().copied()).unwrap_or(1);
                self.screen.goto(i64::from(row) - 1, i64::from(col) - 1);
            }
            'I' => {
                for _ in 0..first(1).max(1) {
                    self.screen.tab();
                }
            }
            'J' => self.screen.erase_display(first(0)),
            'K' => self.screen.erase_line(first(0)),
            '@' => self.screen.insert_blanks(first(1)),
            'P' => self.screen.delete_chars(first(1)),
            // DECSTBM: set scroll region; per spec the cursor homes after.
            'r' => {
                let mut it = params.iter();
                let top = it.next().and_then(|p| p.first().copied()).unwrap_or(1);
                let bottom = it.next().and_then(|p| p.first().copied()).unwrap_or(0);
                let bottom = if bottom == 0 {
                    24_000
                } else {
                    i64::from(bottom)
                };
                self.screen.set_scroll_region(i64::from(top), bottom);
                self.screen.goto(0, 0);
            }
            // SU / SD: scroll region contents without moving the cursor.
            'S' => self.screen.scroll_up(first(1).max(1) as u16),
            'T' => self.screen.scroll_down(first(1).max(1) as u16),
            // RI is ESC M (handled in esc_dispatch), not CSI.
            'M' => {}
            'd' => {
                let row = first(1);
                self.screen.goto(row - 1, i64::from(self.screen.cursor().0));
            }
            'm' => {
                // Flatten all params incl. colon sub-params: 38:2:r:g:b works.
                let flat: Vec<i64> = params.iter().flatten().map(|&v| i64::from(v)).collect();
                self.screen.sgr(&flat);
            }
            'h' | 'l' if private == Some(b'?') => {
                let set = action == 'h';
                for group in params.iter() {
                    for &mode in group {
                        self.screen.set_private_mode(i64::from(mode), set);
                    }
                }
                // ANSI-mode h/l (e.g. 4 = IRM) intentionally unhandled for now.
            }
            _ => {}
        }
    }
}

impl Perform for TermPerformer {
    fn print(&mut self, c: char) {
        self.printed.push(c);
    }

    fn execute(&mut self, byte: u8) {
        self.flush_printed();
        match byte {
            b'\r' => self.screen.carriage_return(),
            b'\n' | 0x0b | 0x0c => self.screen.line_feed(),
            0x08 => self.screen.backspace(),
            b'\t' => self.screen.tab(),
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC 0/2 title etc.: stored later for tab titles (M3 GUI).
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        self.handle_csi(params, intermediates, action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        self.flush_printed();
        match (intermediates.first(), byte) {
            // DECSC / DECRC.
            (None, b'7') => self.screen.save_cursor(),
            (None, b'8') => self.screen.restore_cursor(),
            // RI: reverse index — scroll down at the region top.
            (None, b'M') => self.screen.reverse_index(),
            // IND/NEL could land here later; RIS resets everything.
            // RIS: full reset — clear, home, drop region & alt screen.
            (None, b'c') => {
                let cols = self.screen.cols();
                let rows = self.screen.rows();
                let cap = 10_000;
                self.screen = Screen::with_scrollback_cap(cols, rows, cap);
            }
            _ => {}
        }
    }
}
