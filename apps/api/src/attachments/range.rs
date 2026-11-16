//! A single `Range: bytes=` request against a file of known size.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Selection {
    /// No range, or one that must be ignored: serve the whole file.
    Whole,
    /// Inclusive byte positions, already clamped to the file.
    Slice {
        start: u64,
        end: u64,
    },
    Unsatisfiable,
}

fn number(digits: &str) -> Option<u64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // A number too large for u64 is larger than any file.
    Some(digits.parse().unwrap_or(u64::MAX))
}

/// Units other than `bytes` are ignored. Several ranges, malformed ranges and ranges
/// that start past the end are unsatisfiable: only one range is ever served.
pub(super) fn select(header: Option<&str>, size: u64) -> Selection {
    let Some(header) = header else {
        return Selection::Whole;
    };
    let Some((unit, set)) = header.split_once('=') else {
        return Selection::Unsatisfiable;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return Selection::Whole;
    }
    let Some((first, last)) = set.trim().split_once('-') else {
        return Selection::Unsatisfiable;
    };
    let (first, last) = (first.trim(), last.trim());
    let range = match (number(first), number(last)) {
        // `-n`: the final n bytes.
        (None, Some(count)) if first.is_empty() => {
            if count == 0 || size == 0 {
                return Selection::Unsatisfiable;
            }
            (size.saturating_sub(count), size - 1)
        }
        (Some(start), None) if last.is_empty() => (start, size.saturating_sub(1)),
        (Some(start), Some(end)) => (start, end.min(size.saturating_sub(1))),
        _ => return Selection::Unsatisfiable,
    };
    let (start, end) = range;
    if start >= size || start > end {
        return Selection::Unsatisfiable;
    }
    Selection::Slice { start, end }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(start: u64, end: u64) -> Selection {
        Selection::Slice { start, end }
    }

    #[test]
    fn forms_are_clamped_to_the_file() {
        assert_eq!(select(None, 10), Selection::Whole);
        assert_eq!(select(Some("bytes=0-3"), 10), slice(0, 3));
        assert_eq!(select(Some("bytes=4-"), 10), slice(4, 9));
        assert_eq!(select(Some("bytes=-3"), 10), slice(7, 9));
        assert_eq!(select(Some("bytes=-30"), 10), slice(0, 9));
        assert_eq!(select(Some("bytes=8-99"), 10), slice(8, 9));
        assert_eq!(select(Some("BYTES= 2 - 5 "), 10), slice(2, 5));
        assert_eq!(
            select(Some("bytes=0-99999999999999999999999"), 10),
            slice(0, 9)
        );
    }

    #[test]
    fn unsatisfiable_and_malformed_ranges_are_rejected() {
        for value in [
            "bytes=10-",
            "bytes=10-12",
            "bytes=5-2",
            "bytes=-0",
            "bytes=-",
            "bytes=",
            "bytes=+1-2",
            "bytes=0-1,3-4",
            "bytes=a-b",
            "bytes=1",
            "nonsense",
        ] {
            assert_eq!(select(Some(value), 10), Selection::Unsatisfiable, "{value}");
        }
        assert_eq!(select(Some("bytes=0-"), 0), Selection::Unsatisfiable);
        assert_eq!(select(Some("bytes=-1"), 0), Selection::Unsatisfiable);
    }

    #[test]
    fn unknown_units_are_ignored() {
        assert_eq!(select(Some("items=0-1"), 10), Selection::Whole);
    }
}
