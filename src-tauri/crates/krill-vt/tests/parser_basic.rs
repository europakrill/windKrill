use krill_vt::{ParserConfig, VtParser};

/// Minimal Perform sink that records printable chars into a String.
struct Sink {
    printed: String,
}

impl vte::Perform for Sink {
    fn print(&mut self, c: char) {
        self.printed.push(c);
    }
    fn execute(&mut self, byte: u8) {
        if byte == b'\n' {
            self.printed.push('\n');
        }
    }
}

#[test]
fn parser_passes_plain_text_through() {
    let mut p = VtParser::new(ParserConfig::default());
    let mut sink = Sink {
        printed: String::new(),
    };
    p.advance(b"hello world", &mut sink);
    assert_eq!(sink.printed, "hello world");
}

#[test]
fn parser_strips_sgr_sequences() {
    let mut p = VtParser::new(ParserConfig::default());
    let mut sink = Sink {
        printed: String::new(),
    };
    // red bold text then reset
    p.advance(b"\x1b[1;31mOK\x1b[0m", &mut sink);
    assert_eq!(sink.printed, "OK");
}
