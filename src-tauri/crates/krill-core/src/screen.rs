/// One terminal cell: glyph + attributes.
/// M0 stub; grows with truecolor/emoji/wide-char support in M1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<u8>,
    pub bg: Option<u8>,
    pub bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self { ch: ' ', fg: None, bg: None, bold: false }
    }
}

/// Screen grid model.
///
/// M0 scope: fixed-size grid with basic put/print semantics so the
/// parser->screen pipeline is exercisable in tests. Infinite scrollback,
/// logical blocks, folding and timestamps land in M1+.
pub struct Screen {
    cols: u16,
    rows: u16,
    grid: Vec<Cell>,
    cursor_row: u16,
    cursor_col: u16,
}

impl Screen {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            grid: vec![Cell::default(); usize::from(cols) * usize::from(rows)],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cursor(&self) -> (u16, u16) {
        (self.cursor_col, self.cursor_row)
    }

    /// Write a char at the cursor, advancing it. Wraps at the right margin.
    pub fn put(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.line_feed();
        }
        let idx = usize::from(self.cursor_row) * usize::from(self.cols)
            + usize::from(self.cursor_col);
        self.grid[idx] = Cell { ch, ..Cell::default() };
        self.cursor_col += 1;
    }

    /// Newline semantics (no implicit carriage return; caller decides).
    pub fn line_feed(&mut self) {
        self.cursor_row += 1;
        if self.cursor_row >= self.rows {
            self.cursor_row = self.rows - 1;
            // M1: scroll up + push top line into scrollback ring.
            self.scroll_up();
        }
    }

    /// Move cursor to column 0 of the current row (CR).
    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    pub fn text(&self) -> String {
        self.grid
            .chunks(usize::from(self.cols))
            .map(|row| row.iter().map(|c| c.ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn scroll_up(&mut self) {
        self.grid.drain(..usize::from(self.cols));
        self.grid.resize(
            usize::from(self.cols) * usize::from(self.rows),
            Cell::default(),
        );
    }
}
