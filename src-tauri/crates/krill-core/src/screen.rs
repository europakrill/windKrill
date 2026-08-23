/// Terminal color representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    /// 0-7 standard, 8-15 bright, 16-255 xterm palette.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Character rendition attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

/// One terminal cell: glyph + attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            attrs: Attrs::default(),
        }
    }
}

/// Tracked DEC private modes (CSI ? n h/l).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modes {
    pub mouse_normal: bool,
    pub mouse_button: bool,
    pub mouse_any: bool,
    /// SGR extended mouse coordinates (mode 1006).
    pub mouse_sgr: bool,
    /// Bracketed paste (mode 2004).
    pub bracketed_paste: bool,
}

/// Screen grid model with scrollback.
///
/// Owns the grid, cursor, active rendition and scrolled-off history.
/// Receives parsed semantics either directly (`put`, `goto`, `sgr`, ...)
/// or via `TermPerformer` (the `vte::Perform` bridge).
pub struct Screen {
    cols: u16,
    rows: u16,
    grid: Vec<Cell>,
    scrollback: std::collections::VecDeque<Vec<Cell>>,
    scrollback_cap: usize,
    cursor_row: u16,
    cursor_col: u16,
    attrs: Attrs,
    modes: Modes,
}

impl Screen {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_scrollback_cap(cols, rows, 10_000)
    }

    pub fn with_scrollback_cap(cols: u16, rows: u16, cap: usize) -> Self {
        Self {
            cols,
            rows,
            grid: vec![Cell::default(); usize::from(cols) * usize::from(rows)],
            scrollback: std::collections::VecDeque::with_capacity(cap.min(1024)),
            scrollback_cap: cap,
            cursor_row: 0,
            cursor_col: 0,
            attrs: Attrs::default(),
            modes: Modes::default(),
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

    pub fn attrs(&self) -> &Attrs {
        &self.attrs
    }

    pub fn modes(&self) -> &Modes {
        &self.modes
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// A scrolled-off line (0 = oldest retained).
    pub fn scrollback_line(&self, idx: usize) -> Option<&[Cell]> {
        self.scrollback.get(idx).map(|r| r.as_slice())
    }

    pub fn cell(&self, row: u16, col: u16) -> Option<&Cell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        Some(&self.grid[usize::from(row) * usize::from(self.cols) + usize::from(col)])
    }

    /// Write a char at the cursor, advancing it. Wraps at the right margin (DECAWM).
    pub fn put(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.line_feed();
        }
        let idx = self.idx(self.cursor_row, self.cursor_col);
        self.grid[idx] = Cell {
            ch,
            attrs: self.attrs,
        };
        self.cursor_col += 1;
    }

    /// Line feed; scrolls (pushing into scrollback) at the bottom margin.
    pub fn line_feed(&mut self) {
        self.cursor_row += 1;
        if self.cursor_row >= self.rows {
            self.cursor_row = self.rows - 1;
            self.scroll_up();
        }
    }

    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        self.cursor_col = self.cursor_col.saturating_sub(1);
    }

    /// Horizontal tab: advance to the next multiple of 8.
    pub fn tab(&mut self) {
        let next = (self.cursor_col / 8 + 1) * 8;
        self.cursor_col = next.min(self.cols - 1);
    }

    /// Absolute cursor position, 0-based, clamped to the grid.
    pub fn goto(&mut self, row: i64, col: i64) {
        self.cursor_row = clamp_idx(row, self.rows);
        self.cursor_col = clamp_idx(col, self.cols);
    }

    /// Relative cursor movement, clamped to the grid.
    pub fn move_cursor(&mut self, drow: i64, dcol: i64) {
        let (r, c) = {
            let (c0, r0) = self.cursor();
            (i64::from(r0) + drow, i64::from(c0) + dcol)
        };
        self.goto(r, c);
    }

    /// Erase in display. `0` below, `1` above, `2` (and others) entire screen.
    pub fn erase_display(&mut self, mode: i64) {
        let blank = self.blank();
        match mode {
            0 => {
                self.erase_line_right();
                let start = self.idx(self.cursor_row + 1, 0);
                for c in &mut self.grid[start..] {
                    *c = blank;
                }
            }
            1 => {
                self.erase_line_left();
                let end = self.idx(self.cursor_row, 0);
                for c in &mut self.grid[..end] {
                    *c = blank;
                }
            }
            _ => {
                for c in &mut self.grid[..] {
                    *c = blank;
                }
            }
        }
    }

    /// Erase in line. `0` right, `1` left, `2` whole line.
    pub fn erase_line(&mut self, mode: i64) {
        let blank = self.blank();
        match mode {
            0 => {
                let base = self.idx(self.cursor_row, self.cursor_col);
                let line_end = self.idx(self.cursor_row, self.cols - 1);
                for c in &mut self.grid[base..=line_end] {
                    *c = blank;
                }
            }
            1 => {
                let base = self.idx(self.cursor_row, 0);
                let end = self.idx(self.cursor_row, self.cursor_col);
                for c in &mut self.grid[base..end] {
                    *c = blank;
                }
            }
            _ => {
                let base = usize::from(self.cursor_row) * usize::from(self.cols);
                for c in &mut self.grid[base..base + usize::from(self.cols)] {
                    *c = blank;
                }
            }
        }
    }

    /// Apply an SGR sequence given its flattened numeric parameters
    /// (handles both `38;2;r;g;b` and `38:2:r:g:b` forms).
    pub fn sgr(&mut self, flat: &[i64]) {
        if flat.is_empty() {
            self.attrs = Attrs::default();
            return;
        }
        let mut i = 0;
        while i < flat.len() {
            match flat[i] {
                0 => self.attrs = Attrs::default(),
                1 => self.attrs.bold = true,
                3 => self.attrs.italic = true,
                4 => self.attrs.underline = true,
                7 => self.attrs.reverse = true,
                22 => self.attrs.bold = false,
                23 => self.attrs.italic = false,
                24 => self.attrs.underline = false,
                27 => self.attrs.reverse = false,
                30..=37 => self.attrs.fg = Some(Color::Indexed((flat[i] - 30) as u8)),
                39 => self.attrs.fg = None,
                40..=47 => self.attrs.bg = Some(Color::Indexed((flat[i] - 40) as u8)),
                49 => self.attrs.bg = None,
                90..=97 => self.attrs.fg = Some(Color::Indexed((flat[i] - 90 + 8) as u8)),
                100..=107 => self.attrs.bg = Some(Color::Indexed((flat[i] - 100 + 8) as u8)),
                38 | 48 => {
                    let target = if flat[i] == 38 {
                        SetTarget::Fg
                    } else {
                        SetTarget::Bg
                    };
                    match flat.get(i + 1) {
                        Some(5) => {
                            if let Some(&n) = flat.get(i + 2) {
                                let color = Color::Indexed(n.clamp(0, 255) as u8);
                                self.set_color(target, color);
                            }
                            i += 2;
                        }
                        Some(2) => {
                            let r = flat.get(i + 2).copied().unwrap_or(0).clamp(0, 255) as u8;
                            let g = flat.get(i + 3).copied().unwrap_or(0).clamp(0, 255) as u8;
                            let b = flat.get(i + 4).copied().unwrap_or(0).clamp(0, 255) as u8;
                            self.set_color(target, Color::Rgb(r, g, b));
                            i += 4;
                        }
                        _ => i += 1,
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// DEC private mode set/reset. `set = true` for `h`, `false` for `l`.
    pub fn set_private_mode(&mut self, mode: i64, set: bool) {
        match mode {
            9 | 1000 => self.modes.mouse_normal = set,
            1002 => self.modes.mouse_button = set,
            1003 => self.modes.mouse_any = set,
            1006 => self.modes.mouse_sgr = set,
            2004 => self.modes.bracketed_paste = set,
            _ => {}
        }
    }

    /// Insert `n` blanks under the cursor, shifting the rest right.
    pub fn insert_blanks(&mut self, n: i64) {
        let blank = self.blank();
        let row_base = usize::from(self.cursor_row) * usize::from(self.cols);
        let col = usize::from(self.cursor_col);
        let n = (n.max(0) as usize).min(self.cols as usize - col);
        let width = usize::from(self.cols);
        let row = &mut self.grid[row_base..row_base + width];
        row.copy_within(col..width - n, col + n);
        for c in &mut row[col..col + n] {
            *c = blank;
        }
    }

    /// Delete `n` characters under the cursor, shifting the rest left.
    pub fn delete_chars(&mut self, n: i64) {
        let blank = self.blank();
        let row_base = usize::from(self.cursor_row) * usize::from(self.cols);
        let col = usize::from(self.cursor_col);
        let n = (n.max(0) as usize).min(self.cols as usize - col);
        let width = usize::from(self.cols);
        let row = &mut self.grid[row_base..row_base + width];
        row.copy_within(col + n..width, col);
        for c in &mut row[width - n..width] {
            *c = blank;
        }
    }

    pub fn text(&self) -> String {
        self.grid
            .chunks(usize::from(self.cols))
            .map(|row| row.iter().map(|c| c.ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn set_color(&mut self, target: SetTarget, color: Color) {
        match target {
            SetTarget::Fg => self.attrs.fg = Some(color),
            SetTarget::Bg => self.attrs.bg = Some(color),
        }
    }

    fn blank(&self) -> Cell {
        Cell {
            ch: ' ',
            attrs: self.attrs,
        }
    }

    fn erase_line_right(&mut self) {
        let blank = self.blank();
        let base = self.idx(self.cursor_row, self.cursor_col);
        let line_end = self.idx(self.cursor_row, self.cols - 1);
        for c in &mut self.grid[base..=line_end] {
            *c = blank;
        }
    }

    fn erase_line_left(&mut self) {
        let blank = self.blank();
        let base = self.idx(self.cursor_row, 0);
        let end = self.idx(self.cursor_row, self.cursor_col);
        for c in &mut self.grid[base..end] {
            *c = blank;
        }
    }

    fn idx(&self, row: u16, col: u16) -> usize {
        usize::from(row) * usize::from(self.cols) + usize::from(col)
    }

    fn scroll_up(&mut self) {
        let top: Vec<Cell> = self.grid[..usize::from(self.cols)].to_vec();
        self.scrollback.push_back(top);
        while self.scrollback.len() > self.scrollback_cap {
            self.scrollback.pop_front();
        }
        self.grid.drain(..usize::from(self.cols));
        self.grid.resize(
            usize::from(self.cols) * usize::from(self.rows),
            Cell::default(),
        );
    }
}

enum SetTarget {
    Fg,
    Bg,
}

fn clamp_idx(v: i64, max: u16) -> u16 {
    v.clamp(0, i64::from(max - 1)) as u16
}
