//! Byte-offset editing of JSON-like sources (configs, tsconfig) that keeps
//! comments and formatting: a comment- and string-blind shape of the source
//! to find delimiters on, plus the helpers that walk it.

use anyhow::{bail, ensure, Result};

pub(crate) fn is_top_level_position(shape: &str, start: usize, end: usize) -> bool {
    let mut depth = 0_i32;
    for byte in shape.as_bytes()[start..end].iter().copied() {
        match byte {
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            _ => {},
        }
    }
    depth == 0
}

pub(crate) fn matching_delimiter(
    shape: &str,
    open: usize,
    opening: u8,
    closing: u8,
) -> Result<usize> {
    ensure!(
        shape.as_bytes().get(open) == Some(&opening),
        "delimiter does not start with expected token"
    );
    let mut depth = 0_u32;
    for (offset, byte) in shape.as_bytes()[open..].iter().copied().enumerate() {
        if byte == opening {
            depth += 1;
        } else if byte == closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Ok(open + offset);
            }
        }
    }
    bail!("unclosed delimiter in configuration")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShapeState {
    Code,
    /// Inside a `'` or `"` string; holds the closing quote.
    Quoted(u8),
    Template,
    LineComment,
    BlockComment,
}

/// Same-length copy of `source` where comments and string contents are
/// blanked to spaces (line breaks kept), so delimiters and keys can be found
/// by byte offset. With `preserve_string_words`, identifier-like characters of
/// `'`/`"` strings are kept so quoted property names still match.
pub(crate) fn code_shape(source: &str, preserve_string_words: bool) -> String {
    let bytes = source.as_bytes();
    let mut shape = bytes.to_vec();
    let mut state = ShapeState::Code;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            ShapeState::Code => match (byte, next) {
                (b'/', Some(b'/')) => {
                    shape[index..index + 2].fill(b' ');
                    state = ShapeState::LineComment;
                    index += 1;
                },
                (b'/', Some(b'*')) => {
                    shape[index..index + 2].fill(b' ');
                    state = ShapeState::BlockComment;
                    index += 1;
                },
                (b'\'' | b'"', _) => {
                    shape[index] = b' ';
                    state = ShapeState::Quoted(byte);
                },
                (b'`', _) => {
                    shape[index] = b' ';
                    state = ShapeState::Template;
                },
                _ => {},
            },
            ShapeState::Quoted(closing) => {
                if escaped {
                    shape[index] = b' ';
                    escaped = false;
                } else if byte == b'\\' {
                    shape[index] = b' ';
                    escaped = true;
                } else if byte == closing {
                    shape[index] = b' ';
                    state = ShapeState::Code;
                } else if !preserve_string_words || !is_preserved_string_byte(byte) {
                    shape[index] = b' ';
                }
            },
            ShapeState::Template => {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'`' {
                    state = ShapeState::Code;
                }
                if !is_line_break(byte) {
                    shape[index] = b' ';
                }
            },
            ShapeState::LineComment => {
                if is_line_break(byte) {
                    state = ShapeState::Code;
                } else {
                    shape[index] = b' ';
                }
            },
            ShapeState::BlockComment => {
                if byte == b'*' && next == Some(b'/') {
                    shape[index..index + 2].fill(b' ');
                    state = ShapeState::Code;
                    index += 1;
                } else if !is_line_break(byte) {
                    shape[index] = b' ';
                }
            },
        }
        index += 1;
    }
    String::from_utf8(shape).unwrap_or_else(|_| source.to_owned())
}

/// String bytes `code_shape` keeps when preserving words: identifier and
/// package-name characters, plus line breaks.
fn is_preserved_string_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(byte, b'_' | b'-' | b'$' | b'@' | b'.' | b'/')
        || is_line_break(byte)
}

pub(crate) fn is_line_break(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

pub(crate) fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'-')
}

/// Index of the last code byte strictly between `open` and `close`: not
/// whitespace and not inside a comment (a string's closing quote counts).
/// `None` when the span holds only whitespace and comments. `open` must be a
/// code position, such as a delimiter found on the `code_shape`.
pub(crate) fn last_code_byte(source: &str, open: usize, close: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut last = None;
    let mut index = open + 1;
    while index < close {
        match (bytes[index], bytes.get(index + 1).copied()) {
            (b'/', Some(b'/')) => {
                while index < close && !matches!(bytes[index], b'\n' | b'\r') {
                    index += 1;
                }
            },
            (b'/', Some(b'*')) => {
                index += 2;
                while index < close
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    index += 1;
                }
                index += 2;
            },
            (quote @ (b'"' | b'\''), _) => {
                index += 1;
                while index < close && bytes[index] != quote {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                last = Some(index.min(close - 1));
                index += 1;
            },
            (byte, _) => {
                if !byte.is_ascii_whitespace() {
                    last = Some(index);
                }
                index += 1;
            },
        }
    }
    last
}
