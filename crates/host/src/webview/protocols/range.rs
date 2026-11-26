//! HTTP `Range` request parsing (RFC 9110 section 14) for the `asset://`
//! protocol. Only a single `bytes` range is honoured; anything else the
//! server may ignore is served as the full representation.

/// What a `Range` header asks of a representation `len` bytes long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ByteRange {
    /// No (usable) range: serve the whole file.
    Full,
    /// Inclusive byte range, already clamped to the file.
    Partial { start: u64, end: u64 },
    /// Valid syntax but no byte of it exists: answer 416.
    Unsatisfiable,
}

/// Parse `header` against a file of `len` bytes. Unknown units, several
/// ranges and malformed specs are ignored (`Full`), as RFC 9110 allows.
pub(super) fn parse(header: Option<&str>, len: u64) -> ByteRange {
    let Some(header) = header else {
        return ByteRange::Full;
    };
    let Some((unit, spec)) = header.trim().split_once('=') else {
        return ByteRange::Full;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return ByteRange::Full;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return ByteRange::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        // Suffix range: the last `n` bytes.
        let Some(suffix) = parse_digits(last) else {
            return ByteRange::Full;
        };
        if suffix == 0 || len == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Partial {
            start: len.saturating_sub(suffix),
            end: len - 1,
        };
    }
    let Some(start) = parse_digits(first) else {
        return ByteRange::Full;
    };
    let end = if last.is_empty() {
        u64::MAX
    } else {
        match parse_digits(last) {
            Some(end) if end >= start => end,
            _ => return ByteRange::Full,
        }
    };
    if start >= len {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Partial {
        start,
        end: end.min(len - 1),
    }
}

/// Plain ASCII digits only: `u64::from_str` would also take a leading `+`.
fn parse_digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{parse, ByteRange};

    fn partial(start: u64, end: u64) -> ByteRange {
        ByteRange::Partial { start, end }
    }

    #[test]
    fn absent_or_ignorable_ranges_serve_the_full_file() {
        for header in [
            None,
            Some(""),
            Some("bytes"),
            Some("items=0-1"),
            Some("bytes=0-1,4-5"),
            Some("bytes=abc"),
            Some("bytes=5-2"),
            Some("bytes=+1-2"),
            Some("bytes=1-+2"),
            Some("bytes=-"),
            Some("bytes=--1"),
        ] {
            assert_eq!(parse(header, 100), ByteRange::Full, "{header:?}");
        }
    }

    #[test]
    fn bounded_open_and_suffix_ranges() {
        assert_eq!(parse(Some("bytes=0-0"), 100), partial(0, 0));
        assert_eq!(parse(Some("bytes=10-19"), 100), partial(10, 19));
        assert_eq!(parse(Some("BYTES = 10 - 19 "), 100), partial(10, 19));
        assert_eq!(parse(Some("bytes=10-"), 100), partial(10, 99));
        assert_eq!(parse(Some("bytes=90-500"), 100), partial(90, 99));
        assert_eq!(parse(Some("bytes=-10"), 100), partial(90, 99));
        assert_eq!(parse(Some("bytes=-500"), 100), partial(0, 99));
        assert_eq!(
            parse(Some("bytes=0-18446744073709551615"), 100),
            partial(0, 99)
        );
    }

    #[test]
    fn ranges_past_the_end_are_unsatisfiable() {
        assert_eq!(parse(Some("bytes=100-"), 100), ByteRange::Unsatisfiable);
        assert_eq!(parse(Some("bytes=200-300"), 100), ByteRange::Unsatisfiable);
        assert_eq!(parse(Some("bytes=-0"), 100), ByteRange::Unsatisfiable);
        assert_eq!(parse(Some("bytes=0-"), 0), ByteRange::Unsatisfiable);
        assert_eq!(parse(Some("bytes=-5"), 0), ByteRange::Unsatisfiable);
        // Too large for u64: malformed, not a huge range.
        assert_eq!(
            parse(Some("bytes=99999999999999999999-"), 100),
            ByteRange::Full
        );
    }
}
