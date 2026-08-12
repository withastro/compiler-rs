//! Byte offsets to UTF-16 code units, the unit JS consumers index strings in.

/// Records only the multibyte characters, so ASCII documents cost nothing.
pub(crate) struct Utf16Index {
    marks: Vec<Mark>,
}

struct Mark {
    byte: u32,
    utf16: u32,
    len_utf8: u32,
    len_utf16: u32,
}

impl Utf16Index {
    pub(crate) fn new(text: &str) -> Self {
        let mut marks = Vec::new();
        if text.is_ascii() {
            return Self { marks };
        }
        let mut utf16 = 0u32;
        for (byte, ch) in text.char_indices() {
            let len_utf8 = ch.len_utf8() as u32;
            let len_utf16 = ch.len_utf16() as u32;
            if len_utf8 > 1 {
                marks.push(Mark {
                    byte: byte as u32,
                    utf16,
                    len_utf8,
                    len_utf16,
                });
            }
            utf16 += len_utf16;
        }
        Self { marks }
    }

    pub(crate) fn convert(&self, byte: u32) -> u32 {
        if self.marks.is_empty() {
            return byte;
        }
        let index = self.marks.partition_point(|mark| mark.byte <= byte);
        if index == 0 {
            return byte;
        }
        let mark = &self.marks[index - 1];
        if byte < mark.byte + mark.len_utf8 {
            // Inside a character: clamp to its start rather than splitting it.
            return mark.utf16;
        }
        mark.utf16 + mark.len_utf16 + (byte - mark.byte - mark.len_utf8)
    }
}

#[cfg(test)]
mod tests {
    use super::Utf16Index;

    #[test]
    fn ascii_offsets_are_unchanged() {
        let index = Utf16Index::new("<p>hi</p>");
        assert_eq!(index.convert(0), 0);
        assert_eq!(index.convert(5), 5);
        assert_eq!(index.convert(9), 9);
    }

    #[test]
    fn multibyte_characters_shrink_offsets() {
        // "π" is two bytes and one code unit; "🦄" is four bytes and two.
        let index = Utf16Index::new("aπb🦄c");
        assert_eq!(index.convert(0), 0);
        assert_eq!(index.convert(1), 1);
        assert_eq!(index.convert(3), 2);
        assert_eq!(index.convert(4), 3);
        assert_eq!(index.convert(8), 5);
        assert_eq!(index.convert(9), 6);
    }

    #[test]
    fn offsets_inside_a_character_clamp_to_its_start() {
        let index = Utf16Index::new("aπb");
        assert_eq!(index.convert(2), 1);
    }
}
