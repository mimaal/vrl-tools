//! A route that can never be taken.
//!
//! An `exclusive_route` tries its routes in order and sends each event down
//! the first whose condition holds (`src/transforms/exclusive_route/transform.rs`:
//! a `for` over the routes that returns at the first match). So the order is
//! part of the meaning, and it is easy to get wrong without noticing — more
//! so when the routes are written across files and the order is the order
//! the files merge in ([`crate::config`]).
//!
//! The mistake this looks for is one route hiding another: the earlier
//! condition holds every time the later one does, so the later route gets
//! nothing. That is decidable in general only by the compiler and a solver.
//! It is decided here for one shape, textually, and for nothing else:
//!
//! > both conditions are `==` comparisons joined by `&&`, each side a path or
//! > a literal, and every comparison of the earlier one is among the later
//! > one's.
//!
//! `.vendor == "acme"` before `.vendor == "acme" && .kind == "fw"` is that
//! shape: whatever satisfies the second satisfies the first, which is tried
//! first. Anything else — an `||`, a function call, a `!=`, parentheses, a
//! condition that is not VRL — is not read at all, and nothing is said. The
//! point of being this narrow is that there are no false positives: a `==`
//! between paths and literals cannot fail and has no effects, so the
//! reasoning needs nothing the text does not show.
//!
//! This reads VRL without compiling it, like [`crate::lookups`], and for the
//! same reason answers only what the text settles.

/// A later route no event can reach, and the earlier one taking its events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shadow {
    /// Index of the route that never matches.
    pub later: usize,
    /// Index of the nearest route before it that matches whenever it would.
    pub earlier: usize,
}

/// The routes of `conditions` — in evaluation order, `None` for a condition
/// that is not a VRL string — that an earlier route hides.
#[must_use]
pub fn shadowed(conditions: &[Option<&str>]) -> Vec<Shadow> {
    let read: Vec<Option<Vec<Comparison>>> = conditions
        .iter()
        .map(|condition| condition.and_then(conjunction))
        .collect();

    let mut found = Vec::new();
    for (later, wanted) in read.iter().enumerate() {
        let Some(wanted) = wanted else { continue };
        // The nearest one, so the message points at the route somebody would
        // move, and each hidden route is reported once.
        let earlier = (0..later).rev().find(|&earlier| {
            read[earlier]
                .as_ref()
                .is_some_and(|needed| needed.iter().all(|comparison| wanted.contains(comparison)))
        });
        if let Some(earlier) = earlier {
            found.push(Shadow { later, earlier });
        }
    }
    found
}

/// One `a == b`, with its sides in a fixed order so `1 == .x` is `.x == 1`.
type Comparison = (String, String);

/// A condition as the set of comparisons it requires, when it is nothing but
/// `==` comparisons joined by `&&`.
fn conjunction(condition: &str) -> Option<Vec<Comparison>> {
    let mut comparisons = Vec::new();
    for part in split(condition, "&&")? {
        let sides = split(part, "==")?;
        let [left, right] = sides.as_slice() else {
            return None;
        };
        let (left, right) = (operand(left)?, operand(right)?);
        comparisons.push(if left <= right { (left, right) } else { (right, left) });
    }
    Some(comparisons)
}

/// Splits on an operator outside string literals. `None` when a string is
/// left open, which means the text was not understood.
fn split<'a>(text: &'a str, operator: &str) -> Option<Vec<&'a str>> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            at += 1;
            loop {
                match bytes.get(at)? {
                    b'\\' => at += 2,
                    b'"' => break,
                    _ => at += 1,
                }
            }
            at += 1;
        } else if bytes[at..].starts_with(operator.as_bytes()) {
            parts.push(&text[start..at]);
            at += operator.len();
            start = at;
        } else {
            at += 1;
        }
    }
    parts.push(text.get(start..)?);
    Some(parts)
}

/// One side of a comparison, when it is a path or a literal and nothing
/// else. Returned as written, trimmed.
fn operand(text: &str) -> Option<String> {
    let text = text.trim();
    (is_path(text) || is_string(text) || is_number(text) || matches!(text, "true" | "false" | "null"))
        .then(|| text.to_owned())
}

/// `.a`, `.a.b_c`: field names an identifier can spell. A quoted segment, an
/// index or a metadata path is left unread rather than half-read.
fn is_path(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('.') else {
        return false;
    };
    !rest.is_empty()
        && rest.split('.').all(|segment| {
            let mut chars = segment.chars();
            chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// A `"…"` literal that is one string and not a template.
fn is_string(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || text.contains("{{") {
        return false;
    }
    let mut at = 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return at == bytes.len() - 1,
            _ => at += 1,
        }
    }
    false
}

fn is_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let mut parts = digits.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    parts.next().is_none()
        && !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{shadowed, Shadow};

    fn of(conditions: &[&str]) -> Vec<(usize, usize)> {
        let conditions: Vec<Option<&str>> = conditions.iter().map(|c| Some(*c)).collect();
        shadowed(&conditions)
            .into_iter()
            .map(|Shadow { later, earlier }| (later, earlier))
            .collect()
    }

    #[test]
    fn a_broader_route_first_hides_the_narrower_one() {
        assert_eq!(of(&[r#".vendor == "acme""#, r#".vendor == "acme" && .kind == "fw""#]), [(1, 0)]);
    }

    /// The other way round is the correct way to write it.
    #[test]
    fn the_narrower_route_first_hides_nothing() {
        assert!(of(&[r#".vendor == "acme" && .kind == "fw""#, r#".vendor == "acme""#]).is_empty());
    }

    #[test]
    fn the_same_condition_twice_hides_the_second() {
        assert_eq!(of(&[".x == 1", ".y == 2", ".x == 1"]), [(2, 0)]);
    }

    /// Order of the comparisons, order of the sides, and spacing are not
    /// part of what a condition means.
    #[test]
    fn writing_it_differently_does_not_hide_it_from_the_check() {
        assert_eq!(
            of(&[r#""acme"==.vendor"#, r#".kind == "fw"  &&  .vendor == "acme""#]),
            [(1, 0)],
        );
    }

    #[test]
    fn the_nearest_hiding_route_is_the_one_named() {
        assert_eq!(of(&[".a == 1", ".a == 1", ".a == 1 && .b == 2"]), [(1, 0), (2, 1)]);
    }

    #[test]
    fn different_values_and_different_fields_hide_nothing() {
        assert!(of(&[".a == 1", ".a == 2 && .b == 1"]).is_empty());
        assert!(of(&[".a == 1", ".b == 1 && .c == 1"]).is_empty());
        assert!(of(&[r#".a == "1""#, ".a == 1 && .b == 2"]).is_empty(), "a string is not a number");
    }

    /// Everything outside the one shape is left alone, on either side.
    #[test]
    fn anything_but_plain_comparisons_is_not_read() {
        for unread in [
            ".a == 1 || .b == 2",
            "(.a == 1)",
            "!(.a == 1)",
            ".a != 1",
            ".a >= 1",
            "exists(.a)",
            "to_string(.a) == \"1\"",
            ".a == .b + 1",
            ".a[0] == 1",
            ".\"a b\" == 1",
            "%meta == 1",
            ".a == \"{{ b }}\"",
            ".a == 1 # the common case",
            ".a == 1\n.b == 2",
            ".a = 1",
            ".a == 1 == true",
            ".a == \"open",
            "",
        ] {
            assert!(of(&[unread, &format!("{unread} && .z == 9")]).is_empty(), "earlier: {unread}");
            assert!(of(&[".z == 9", unread]).is_empty(), "later: {unread}");
        }
    }

    /// An operator inside a string is text, not an operator.
    #[test]
    fn operators_inside_strings_are_text() {
        assert_eq!(
            of(&[r#".q == "a && b == c""#, r#".q == "a && b == c" && .n == 1"#]),
            [(1, 0)],
        );
        assert!(of(&[r#".q == "a""#, r#".q == "a\" && .n == 1""#]).is_empty());
    }

    #[test]
    fn a_condition_that_is_not_vrl_text_is_skipped() {
        let found = shadowed(&[Some(".a == 1"), None, Some(".a == 1 && .b == 2")]);
        assert_eq!(found, [Shadow { later: 2, earlier: 0 }]);
    }
}
