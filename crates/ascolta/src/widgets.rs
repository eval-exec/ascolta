use unicode_width::UnicodeWidthChar;

/// Cursor and scroll offset for a vertical list.
///
/// Kept separate from ratatui's `ListState` so the same logic works with the
/// custom, viewport-only renderers used for large libraries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListView {
    pub cursor: usize,
    pub offset: usize,
}

impl ListView {
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.offset = 0;
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.reset();
        } else {
            self.cursor = self.cursor.min(len - 1);
            self.offset = self.offset.min(self.cursor);
        }
    }

    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.reset();
            return;
        }
        self.cursor = self
            .cursor
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
    }

    pub fn page(&mut self, delta: isize, len: usize, page: usize) {
        let page = page.max(1) as isize;
        self.move_by(delta.saturating_mul(page), len);
    }

    pub fn first(&mut self) {
        self.cursor = 0;
    }

    pub fn last(&mut self, len: usize) {
        self.cursor = len.saturating_sub(1);
    }

    /// Adjusts the offset so the cursor is inside a viewport of `height` rows.
    pub fn ensure_visible(&mut self, len: usize, height: usize) {
        if len == 0 {
            self.reset();
            return;
        }
        if height == 0 {
            return;
        }
        self.cursor = self.cursor.min(len - 1);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset.saturating_add(height) {
            self.offset = self.cursor + 1 - height;
        }
        self.offset = self.offset.min(len.saturating_sub(height));
    }

    pub fn selected(&self, len: usize) -> Option<usize> {
        (self.cursor < len).then_some(self.cursor)
    }
}

/// Truncates or space-pads `text` to exactly `width` terminal cells.
///
/// Padding with character counts would misalign rows containing wide glyphs
/// (CJK, emoji); this walks display widths instead.
pub fn fit(text: &str, width: usize) -> String {
    let mut out = String::with_capacity(width);
    let mut used = 0;
    for character in text.chars().map(display_char) {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > width {
            break;
        }
        out.push(character);
        used += character_width;
    }
    while used < width {
        out.push(' ');
        used += 1;
    }
    out
}

/// Right-aligned variant of [`fit`].
pub fn fit_right(text: &str, width: usize) -> String {
    let text_width = text
        .chars()
        .map(display_char)
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
        .sum::<usize>();
    if text_width >= width {
        return fit(text, width);
    }
    let mut out = String::with_capacity(width);
    for _ in 0..(width - text_width) {
        out.push(' ');
    }
    out.extend(text.chars().map(display_char));
    out
}

/// Control characters would move the cursor or alter the line unexpectedly.
fn display_char(character: char) -> char {
    if character.is_control() {
        ' '
    } else {
        character
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_pads_and_truncates() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 3), "abc");
        assert_eq!(fit("界", 1), " ");
        assert_eq!(fit("界", 2), "界");
        assert_eq!(fit("a\tb", 3), "a b");
        assert_eq!(fit("", 0), "");
    }

    #[test]
    fn fit_always_fills_the_requested_cell_width() {
        for text in [
            "",
            "abc",
            "日本語",
            "🎵🎶",
            "e\u{301}",
            "a\tb",
            "界a界",
            "\u{200b}",
        ] {
            for width in 0..12 {
                assert_eq!(
                    unicode_width::UnicodeWidthStr::width(fit(text, width).as_str()),
                    width,
                    "text {text:?} width {width}"
                );
            }
        }
    }

    #[test]
    fn fit_right_aligns() {
        assert_eq!(fit_right("42", 4), "  42");
        assert_eq!(fit_right("12345", 4), "1234");
    }

    #[test]
    fn list_view_keeps_cursor_visible() {
        let mut view = ListView::default();
        view.move_by(9, 20);
        view.ensure_visible(20, 5);
        assert_eq!(view.cursor, 9);
        assert_eq!(view.offset, 5);

        view.cursor = 1;
        view.ensure_visible(20, 5);
        assert_eq!(view.offset, 1);

        view.cursor = 19;
        view.ensure_visible(20, 5);
        assert_eq!(view.offset, 15);
    }

    #[test]
    fn list_view_clamps_empty_and_short_lists() {
        let mut view = ListView {
            cursor: 7,
            offset: 3,
        };
        view.clamp(0);
        assert_eq!(view, ListView::default());

        let mut view = ListView {
            cursor: 7,
            offset: 3,
        };
        view.clamp(3);
        assert_eq!(view.cursor, 2);

        view.ensure_visible(3, 10);
        assert_eq!(view.offset, 0);
    }

    #[test]
    fn paging_saturates() {
        let mut view = ListView::default();
        view.page(1, 20, 10);
        assert_eq!(view.cursor, 10);
        view.page(-5, 20, 10);
        assert_eq!(view.cursor, 0);
        view.page(1, 0, 10);
        assert_eq!(view.cursor, 0);
    }
}
