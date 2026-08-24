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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    pub mouse_normal: bool,
    pub mouse_button: bool,
    pub mouse_any: bool,
    /// SGR extended mouse coordinates (mode 1006).
    pub mouse_sgr: bool,
    /// Bracketed paste (mode 2004).
    pub bracketed_paste: bool,
    /// Alternate screen buffer active (modes 47/1047/1049).
    pub alt_screen: bool,
    /// Auto-wrap (DECAWM, mode 7). On by default.
    pub autowrap: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            mouse_normal: false,
            mouse_button: false,
            mouse_any: false,
            mouse_sgr: false,
            bracketed_paste: false,
            alt_screen: false,
            autowrap: true,
        }
    }
}

/// Hard allocation limits shared by the screen and local PTY transports.
/// Four million cells keeps one grid bounded to a predictable amount of RAM
/// while remaining far above practical terminal viewport sizes.
pub const MAX_SCREEN_DIMENSION: u16 = 4096;
pub const MAX_SCREEN_CELLS: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSizeError {
    pub cols: u16,
    pub rows: u16,
}

impl std::fmt::Display for ScreenSizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid terminal size {}x{}; dimensions must be 1..={} and total cells <= {}",
            self.cols, self.rows, MAX_SCREEN_DIMENSION, MAX_SCREEN_CELLS
        )
    }
}

impl std::error::Error for ScreenSizeError {}

pub fn validate_screen_size(cols: u16, rows: u16) -> Result<(), ScreenSizeError> {
    let cells = usize::from(cols) * usize::from(rows);
    if cols == 0
        || rows == 0
        || cols > MAX_SCREEN_DIMENSION
        || rows > MAX_SCREEN_DIMENSION
        || cells > MAX_SCREEN_CELLS
    {
        return Err(ScreenSizeError { cols, rows });
    }
    Ok(())
}

/// Screen grid model with scrollback.
///
/// Owns the grid, cursor, active rendition and scrolled-off history.
/// Receives parsed semantics either directly (`put`, `goto`, `sgr`, ...)
/// or via `TermPerformer` (the `vte::Perform` bridge).
#[derive(Clone)]
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
    /// DECSTBM scroll region, 0-based inclusive (top, bottom). `None` = full screen.
    scroll_region: Option<(u16, u16)>,
    /// Alternate-screen buffer (kept while the primary is active).
    alt_grid: Option<Vec<Cell>>,
    /// Saved cursor for DECSC (ESC 7) / DECRC (ESC 8), and for 1049 save.
    saved_cursor: Option<(u16, u16, Attrs)>,
}

impl Screen {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::try_new(cols, rows).expect("screen dimensions must be valid")
    }

    pub fn try_new(cols: u16, rows: u16) -> Result<Self, ScreenSizeError> {
        Self::try_with_scrollback_cap(cols, rows, 10_000)
    }

    pub fn with_scrollback_cap(cols: u16, rows: u16, cap: usize) -> Self {
        Self::try_with_scrollback_cap(cols, rows, cap).expect("screen dimensions must be valid")
    }

    pub fn try_with_scrollback_cap(
        cols: u16,
        rows: u16,
        cap: usize,
    ) -> Result<Self, ScreenSizeError> {
        validate_screen_size(cols, rows)?;
        Ok(Self {
            cols,
            rows,
            grid: vec![Cell::default(); usize::from(cols) * usize::from(rows)],
            scrollback: std::collections::VecDeque::with_capacity(cap.min(1024)),
            scrollback_cap: cap,
            cursor_row: 0,
            cursor_col: 0,
            attrs: Attrs::default(),
            modes: Modes::default(),
            scroll_region: None,
            alt_grid: None,
            saved_cursor: None,
        })
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Resize the visible and inactive alternate grids, preserving the
    /// top-left overlap. Full xterm-style reflow is intentionally deferred.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ScreenSizeError> {
        validate_screen_size(cols, rows)?;
        if cols == self.cols && rows == self.rows {
            return Ok(());
        }

        fn resized_grid(
            source: &[Cell],
            old_cols: u16,
            old_rows: u16,
            new_cols: u16,
            new_rows: u16,
        ) -> Vec<Cell> {
            let mut target = vec![Cell::default(); usize::from(new_cols) * usize::from(new_rows)];
            let copy_rows = old_rows.min(new_rows);
            let copy_cols = old_cols.min(new_cols);
            for row in 0..copy_rows {
                let old_start = usize::from(row) * usize::from(old_cols);
                let new_start = usize::from(row) * usize::from(new_cols);
                target[new_start..new_start + usize::from(copy_cols)]
                    .copy_from_slice(&source[old_start..old_start + usize::from(copy_cols)]);
            }
            target
        }

        let old_cols = self.cols;
        let cursor_was_wrap_pending = self.cursor_col >= old_cols;
        let saved_cursor = self.saved_cursor;
        self.grid = resized_grid(&self.grid, self.cols, self.rows, cols, rows);
        if let Some(alt) = self.alt_grid.take() {
            self.alt_grid = Some(resized_grid(&alt, self.cols, self.rows, cols, rows));
        }
        self.cols = cols;
        self.rows = rows;
        self.cursor_col = if cursor_was_wrap_pending {
            old_cols.saturating_sub(1).min(cols - 1)
        } else {
            self.cursor_col.min(cols - 1)
        };
        self.cursor_row = self.cursor_row.min(rows - 1);
        if let Some((row, col, attrs)) = saved_cursor {
            let col = if col >= old_cols {
                old_cols.saturating_sub(1).min(cols - 1)
            } else {
                col.min(cols - 1)
            };
            self.saved_cursor = Some((row.min(rows - 1), col, attrs));
        }
        self.scroll_region = None;
        Ok(())
    }

    pub fn cursor(&self) -> (u16, u16) {
        (self.cursor_col, self.cursor_row)
    }

    /// Cursor position clamped to an addressable cell. Internally
    /// `cursor_col == cols` represents delayed auto-wrap after writing the
    /// right margin; terminal reports must not expose that sentinel.
    pub fn visible_cursor(&self) -> (u16, u16) {
        (
            self.cursor_col.min(self.cols - 1),
            self.cursor_row.min(self.rows - 1),
        )
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
        // DECAWM off: the last column is overwritten in place.
        if self.cursor_col >= self.cols && self.modes.autowrap {
            self.cursor_col = 0;
            self.line_feed();
        }
        if self.cursor_col >= self.cols {
            self.cursor_col = self.cols - 1;
        }
        let idx = self.idx(self.cursor_row, self.cursor_col);
        self.grid[idx] = Cell {
            ch,
            attrs: self.attrs,
        };
        self.cursor_col += 1;
    }

    /// Line feed; scrolls (pushing into scrollback) at the bottom margin.
    ///
    /// Inside a DECSTBM region only the region scrolls and no scrollback
    /// entry is produced (region scrolling is application-driven, like vim).
    pub fn line_feed(&mut self) {
        let bottom = self.scroll_bottom();
        if self.cursor_row == bottom {
            self.scroll_up(1);
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }

    /// Reverse index (ESC M / CSI L context): feed up; scrolls down at the top.
    pub fn reverse_index(&mut self) {
        let top = self.scroll_top();
        if self.cursor_row == top {
            self.scroll_down(1);
        } else {
            self.cursor_row -= 1;
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
    ///
    /// While a scroll region is set (DECSTBM), CUP with DECOM-style origin
    /// is NOT implied here; plain CUP stays screen-absolute per xterm.
    pub fn goto(&mut self, row: i64, col: i64) {
        self.cursor_row = clamp_idx(row, self.rows);
        self.cursor_col = clamp_idx(col, self.cols);
    }

    /// Set the DECSTBM scroll region (1-based inclusive params, as received).
    /// Requires top < bottom; otherwise the request is ignored (per spec).
    pub fn set_scroll_region(&mut self, top: i64, bottom: i64) {
        let top = top.clamp(1, i64::from(self.rows)) - 1;
        let bottom = bottom.clamp(1, i64::from(self.rows)) - 1;
        if top < bottom {
            self.scroll_region = Some((top as u16, bottom as u16));
        }
    }

    /// Reset the scroll region to the full screen.
    pub fn reset_scroll_region(&mut self) {
        self.scroll_region = None;
    }

    /// Switch to the alternate screen buffer.
    ///
    /// `save_cursor`: modes 1049/1048 also remember the cursor position
    /// (and rendition) for restoration on return; bare 47/1047 do not.
    pub fn enter_alt_screen(&mut self, save_cursor: bool) {
        if self.modes.alt_screen {
            return;
        }
        if save_cursor {
            self.saved_cursor = Some((self.cursor_row, self.cursor_col, self.attrs));
        }
        let blank = Cell::default();
        let grid = std::mem::replace(
            &mut self.grid,
            vec![blank; usize::from(self.cols) * usize::from(self.rows)],
        );
        self.alt_grid = Some(grid);
        self.modes.alt_screen = true;
    }

    /// Return to the primary screen buffer.
    pub fn exit_alt_screen(&mut self, restore_cursor: bool) {
        if !self.modes.alt_screen {
            return;
        }
        if let Some(grid) = self.alt_grid.take() {
            self.grid = grid;
        }
        self.modes.alt_screen = false;
        if restore_cursor {
            if let Some((r, c, attrs)) = self.saved_cursor.take() {
                self.cursor_row = r.min(self.rows - 1);
                self.cursor_col = c.min(self.cols - 1);
                self.attrs = attrs;
                return;
            }
        }
        // xterm clears the alt screen on entry and homes the cursor on both edges.
        self.erase_display(2);
        self.goto(0, 0);
    }

    /// Save cursor + rendition (DECSC, ESC 7).
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some((self.cursor_row, self.cursor_col, self.attrs));
    }

    /// Restore cursor + rendition (DECRC, ESC 8); no-op if never saved.
    pub fn restore_cursor(&mut self) {
        if let Some((r, c, attrs)) = self.saved_cursor.take() {
            self.cursor_row = r.min(self.rows - 1);
            self.cursor_col = c.min(self.cols - 1);
            self.attrs = attrs;
        }
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
            7 => self.modes.autowrap = set,
            47 => {
                if set {
                    self.enter_alt_screen(false);
                } else {
                    self.exit_alt_screen(false);
                }
            }
            1047 => {
                if set {
                    self.enter_alt_screen(false);
                } else {
                    self.exit_alt_screen(false);
                }
            }
            // 1048 = save/restore cursor only (no buffer swap).
            1048 => {
                if set {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if set {
                    self.save_cursor();
                    self.enter_alt_screen(false);
                } else {
                    self.exit_alt_screen(true);
                }
            }
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

    fn scroll_top(&self) -> u16 {
        self.scroll_region.map_or(0, |(top, _)| top)
    }

    fn scroll_bottom(&self) -> u16 {
        self.scroll_region
            .map_or(self.rows - 1, |(_, bottom)| bottom)
    }

    /// Scroll `n` lines up inside the DECSTBM region. The line leaving the
    /// region enters scrollback only when the region is the full screen.
    pub fn scroll_up(&mut self, n: u16) {
        let (top, bottom) = (self.scroll_top(), self.scroll_bottom());
        let width = usize::from(self.cols);
        let n = n.min(bottom - top) as usize;
        if n == 0 {
            return;
        }
        // Full-screen scroll: the topmost lines go to scrollback —
        // except on the alternate screen, which has no scrollback.
        if top == 0 && bottom == self.rows - 1 && !self.modes.alt_screen {
            for i in 0..n {
                let start = i * width;
                let line = self.grid[start..start + width].to_vec();
                self.scrollback.push_back(line);
            }
            while self.scrollback.len() > self.scrollback_cap {
                self.scrollback.pop_front();
            }
        }
        for _ in 0..n {
            let base = usize::from(top) * width;
            let end = (usize::from(bottom) + 1) * width;
            self.grid.drain(base..base + width);
            self.grid
                .splice(end - width..end - width, vec![Cell::default(); width]);
        }
    }

    /// Scroll `n` lines down inside the DECSTBM region.
    pub fn scroll_down(&mut self, n: u16) {
        let (top, bottom) = (self.scroll_top(), self.scroll_bottom());
        let width = usize::from(self.cols);
        let n = n.min(bottom - top) as usize;
        for _ in 0..n {
            let base = usize::from(top) * width;
            let end = (usize::from(bottom) + 1) * width;
            self.grid
                .splice(end - width..end, vec![Cell::default(); width]);
            self.grid.drain(end - width..end);
            self.grid.splice(base..base, vec![Cell::default(); width]);
        }
    }
}

enum SetTarget {
    Fg,
    Bg,
}

fn clamp_idx(v: i64, max: u16) -> u16 {
    v.clamp(0, i64::from(max - 1)) as u16
}
