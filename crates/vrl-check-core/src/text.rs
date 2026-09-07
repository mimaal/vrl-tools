//! Byte offsets to editor positions.
//!
//! The VRL compiler reports spans as byte offsets into the source. Editors
//! built on the Language Server Protocol — VS Code among them — address text as
//! a zero-based line plus a column counted in **UTF-16 code units**. The two
//! only agree while the source is pure ASCII, and log parsers are full of
//! accented words, arrows and emoji, so the conversion has to be exact or the
//! squiggles land on the wrong characters.

use serde::{Deserialize, Serialize};

/// A zero-based position, with `character` counted in UTF-16 code units.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    #[must_use]
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// A half-open range, in the same coordinates as [`Position`].
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    #[must_use]
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// Converts byte offsets into [`Position`]s for one document.
///
/// Built once per compilation and reused for every span, since a program with
/// twenty diagnostics should not scan the source twenty times.
pub struct LineIndex<'a> {
    source: &'a str,
    /// Byte offset of the first character of each line.
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(offset, _)| offset + 1),
        );

        Self {
            source,
            line_starts,
        }
    }

    /// The position of a byte offset.
    ///
    /// Offsets past the end of the source clamp to the end, and an offset that
    /// lands inside a multi-byte character rounds down to that character's
    /// start. Neither should happen with spans the compiler produces, but a
    /// panic here would take out the whole diagnostic run.
    #[must_use]
    pub fn position(&self, byte_offset: usize) -> Position {
        let offset = self.floor_char_boundary(byte_offset.min(self.source.len()));

        // The line whose start is the last one at or before `offset`.
        let line = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index - 1,
        };

        let line_start = self.line_starts[line];
        let character = self.source[line_start..offset].encode_utf16().count();

        Position {
            line: as_u32(line),
            character: as_u32(character),
        }
    }

    /// The range of a compiler span.
    #[must_use]
    pub fn range(&self, span: std::ops::Range<usize>) -> Range {
        Range {
            start: self.position(span.start),
            end: self.position(span.end),
        }
    }

    fn floor_char_boundary(&self, offset: usize) -> usize {
        let mut offset = offset;
        while offset > 0 && !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }
}

/// Positions in a document that needs more than 4 billion lines are not a
/// problem this extension has.
fn as_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte lengths worth keeping in mind while reading these tests:
    /// `ñ` is 2 bytes and 1 UTF-16 unit, `→` is 3 bytes and 1 unit, and `🚀`
    /// is 4 bytes and 2 units, because it lives outside the BMP and is encoded
    /// as a surrogate pair.
    #[test]
    fn ascii_is_the_easy_case() {
        let source = "abc\ndef\n";
        let index = LineIndex::new(source);

        assert_eq!(index.position(0), Position::new(0, 0));
        assert_eq!(index.position(2), Position::new(0, 2));
        assert_eq!(index.position(4), Position::new(1, 0));
        assert_eq!(index.position(7), Position::new(1, 3));
    }

    #[test]
    fn accented_characters_are_one_unit_wide() {
        let source = ".mensaje = \"año\"";
        let index = LineIndex::new(source);

        let bytes = source.find("año").unwrap();
        assert_eq!(index.position(bytes), Position::new(0, 12));

        // After the ñ: 2 bytes further along, but only 1 column.
        assert_eq!(index.position(bytes + 3), Position::new(0, 14));
    }

    #[test]
    fn arrows_and_cjk_are_one_unit_wide() {
        let source = "x = \"→ 日本語\"";
        let index = LineIndex::new(source);

        let after_arrow = source.find('→').unwrap() + '→'.len_utf8();
        assert_eq!(index.position(after_arrow), Position::new(0, 6));

        let end = source.len();
        assert_eq!(index.position(end), Position::new(0, 11));
    }

    #[test]
    fn astral_plane_characters_are_two_units_wide() {
        let source = "x = \"🚀\"";
        let index = LineIndex::new(source);

        let rocket = source.find('🚀').unwrap();
        assert_eq!(index.position(rocket), Position::new(0, 5));
        // The closing quote sits one character but two UTF-16 units later.
        assert_eq!(index.position(rocket + 4), Position::new(0, 7));
    }

    #[test]
    fn columns_reset_after_a_multibyte_line() {
        let source = "x = \"ñ→🚀\"\ny = 1";
        let index = LineIndex::new(source);

        let second_line = source.find("y =").unwrap();
        assert_eq!(index.position(second_line), Position::new(1, 0));
        assert_eq!(index.position(second_line + 4), Position::new(1, 4));
    }

    #[test]
    fn a_span_becomes_a_range() {
        let source = "x = ñ + 1";
        let index = LineIndex::new(source);
        let start = source.find('ñ').unwrap();

        assert_eq!(
            index.range(start..start + 2),
            Range::new(Position::new(0, 4), Position::new(0, 5)),
        );
    }

    #[test]
    fn offsets_inside_a_character_round_down() {
        let source = "x = 🚀";
        let index = LineIndex::new(source);
        let rocket = source.find('🚀').unwrap();

        // Byte 1 and 2 of the rocket are not character boundaries.
        assert_eq!(index.position(rocket + 1), index.position(rocket));
        assert_eq!(index.position(rocket + 2), index.position(rocket));
    }

    #[test]
    fn offsets_past_the_end_clamp() {
        let source = "x = 1";
        let index = LineIndex::new(source);

        assert_eq!(index.position(999), Position::new(0, 5));
    }

    #[test]
    fn a_trailing_newline_opens_one_more_line() {
        let source = "x = 1\n";
        let index = LineIndex::new(source);

        assert_eq!(index.position(source.len()), Position::new(1, 0));
    }

    #[test]
    fn crlf_does_not_shift_the_next_line() {
        let source = "x = 1\r\ny = 2";
        let index = LineIndex::new(source);
        let second_line = source.find("y =").unwrap();

        assert_eq!(index.position(second_line), Position::new(1, 0));
    }

    #[test]
    fn an_empty_document_has_one_position() {
        let index = LineIndex::new("");

        assert_eq!(index.position(0), Position::new(0, 0));
        assert_eq!(index.position(10), Position::new(0, 0));
    }
}
