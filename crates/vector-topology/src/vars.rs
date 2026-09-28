//! Environment variables in a config, interpolated the way Vector does before
//! it parses anything.
//!
//! Vector reads a config file as text, replaces `$NAME`, `${NAME}` and their
//! `${NAME:-default}` family, and only then hands the result to the TOML or
//! YAML parser (`prepare_input` and `vars::interpolate`, in
//! `src/config/loading/mod.rs` and `src/config/vars.rs`). A file written for
//! that — `max_size = ${BUFFER_SIZE_BYTES}`, unquoted — is not TOML until the
//! variable is replaced, and reading it without doing the same loses every
//! component in it.
//!
//! What the variables hold is on the machine Vector runs on, not the one the
//! editor runs on, so their values are not looked up. A default is used, as
//! Vector would when the variable is unset. Anything else keeps its written
//! form — `${ENV}_logs` stays that, in the component that declares it and in
//! the input that names it, so the two still meet — except where the written
//! form would not parse: a bare TOML value, which is given quotes. `$$` is a
//! literal `$`, as in Vector.
//!
//! Replacing text moves everything after it, and the spans the parsers report
//! are in the replaced text. [`Interpolated::original`] takes an offset back
//! to the file as written, which is where a finding has to point.

/// Which grammar the text is in, for telling where a bare value can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Syntax {
    Yaml,
    Toml,
}

/// A config with its variables replaced, and the way back to the original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interpolated {
    pub text: String,
    edits: Vec<Edit>,
}

/// One replacement: `old` bytes at `original` became `new` bytes at `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edit {
    at: usize,
    new: usize,
    original: usize,
    old: usize,
}

impl Interpolated {
    /// The offset in the file as written of an offset in [`Self::text`].
    ///
    /// An offset inside a replacement maps to the start of what it replaced,
    /// or to its end when it is the replacement's end, so a span covering a
    /// variable covers the variable as written.
    pub fn original(&self, offset: usize) -> usize {
        let before = self.edits.partition_point(|edit| edit.at <= offset);
        let Some(edit) = before.checked_sub(1).map(|index| self.edits[index]) else {
            return offset;
        };
        let end = edit.at + edit.new;
        if offset >= end {
            offset - end + edit.original + edit.old
        } else if offset == edit.at {
            edit.original
        } else {
            edit.original + edit.old
        }
    }
}

/// Replaces the variables in `source` as described in the module docs.
pub(crate) fn interpolate(source: &str, syntax: Syntax) -> Interpolated {
    let bytes = source.as_bytes();
    let bare = match syntax {
        Syntax::Toml => toml_bare(source),
        Syntax::Yaml => Vec::new(),
    };

    let mut text = String::with_capacity(source.len());
    let mut edits = Vec::new();
    let mut copied = 0;
    let mut position = 0;

    while let Some(found) = source[position..].find('$') {
        let start = position + found;
        let Some((end, replacement)) = reference(bytes, start, bare.get(start) == Some(&true))
        else {
            position = start + 1;
            continue;
        };
        position = end;

        let Some(replacement) = replacement else {
            continue;
        };
        text.push_str(&source[copied..start]);
        edits.push(Edit {
            at: text.len(),
            new: replacement.len(),
            original: start,
            old: end - start,
        });
        text.push_str(&replacement);
        copied = end;
    }
    text.push_str(&source[copied..]);

    Interpolated { text, edits }
}

/// The variable reference starting at the `$` at `start`, if one does: where
/// it ends, and what replaces it (`None` for "leave it as written").
///
/// The same three alternatives, in the same order, as Vector's
/// `ENVIRONMENT_VARIABLE_INTERPOLATION_REGEX`:
/// `\$\$ | \$([[:word:].]+) | \$\{([[:word:].]+)(?:(:?-|:?\?)([^}]*))?\}`.
fn reference(bytes: &[u8], start: usize, bare: bool) -> Option<(usize, Option<String>)> {
    let name_end = |from: usize| {
        let mut end = from;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'.')) {
            end += 1;
        }
        end
    };
    let unknown = |name: &[u8]| {
        bare.then(|| format!("\"${{{}}}\"", String::from_utf8_lossy(name)))
    };

    match bytes.get(start + 1)? {
        b'$' => Some((start + 2, Some("$".to_owned()))),
        b'{' => {
            let name_start = start + 2;
            let end = name_end(name_start);
            if end == name_start {
                return None;
            }
            let name = &bytes[name_start..end];
            match bytes.get(end)? {
                b'}' => Some((end + 1, unknown(name))),
                b':' | b'-' | b'?' => {
                    let flag_end = match (bytes[end], bytes.get(end + 1)) {
                        (b':', Some(b'-' | b'?')) => end + 2,
                        (b':', _) => return None,
                        _ => end + 1,
                    };
                    let close = flag_end + bytes[flag_end..].iter().position(|&b| b == b'}')?;
                    let default = bytes[end..flag_end].ends_with(b"-");
                    let replacement = if default {
                        Some(String::from_utf8_lossy(&bytes[flag_end..close]).into_owned())
                    } else {
                        unknown(name)
                    };
                    Some((close + 1, replacement))
                }
                _ => None,
            }
        }
        _ => {
            let end = name_end(start + 1);
            (end > start + 1).then(|| (end, unknown(&bytes[start + 1..end])))
        }
    }
}

/// For each byte of a TOML document, whether it is outside every string and
/// comment — where a `$` can only be the start of a bare value.
///
/// A lexer of four string forms and comments, nothing more. `$` is never
/// valid TOML outside a string, so this is only asked where the file would
/// not parse as written anyway.
fn toml_bare(source: &str) -> Vec<bool> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Bare,
        Comment,
        Basic,
        Literal,
        MultiBasic,
        MultiLiteral,
    }

    let bytes = source.as_bytes();
    let mut bare = vec![false; bytes.len()];
    let mut state = State::Bare;
    let mut index = 0;
    let at = |index: usize, text: &[u8]| bytes[index..].starts_with(text);

    while index < bytes.len() {
        let byte = bytes[index];
        match state {
            State::Bare => {
                bare[index] = true;
                if byte == b'#' {
                    state = State::Comment;
                } else if at(index, b"\"\"\"") {
                    state = State::MultiBasic;
                    index += 3;
                    continue;
                } else if at(index, b"'''") {
                    state = State::MultiLiteral;
                    index += 3;
                    continue;
                } else if byte == b'"' {
                    state = State::Basic;
                } else if byte == b'\'' {
                    state = State::Literal;
                }
            }
            State::Comment => {
                if byte == b'\n' {
                    state = State::Bare;
                }
            }
            State::Basic | State::MultiBasic if byte == b'\\' => {
                index += 2;
                continue;
            }
            State::Basic => {
                if byte == b'"' || byte == b'\n' {
                    state = State::Bare;
                }
            }
            State::Literal => {
                if byte == b'\'' || byte == b'\n' {
                    state = State::Bare;
                }
            }
            State::MultiBasic => {
                if at(index, b"\"\"\"") {
                    state = State::Bare;
                    index += 3;
                    continue;
                }
            }
            State::MultiLiteral => {
                if at(index, b"'''") {
                    state = State::Bare;
                    index += 3;
                    continue;
                }
            }
        }
        index += 1;
    }
    bare
}

#[cfg(test)]
mod tests {
    use super::{interpolate, Syntax};

    fn yaml(source: &str) -> String {
        interpolate(source, Syntax::Yaml).text
    }

    fn toml(source: &str) -> String {
        interpolate(source, Syntax::Toml).text
    }

    /// The cases of Vector's own `vars::tests::interpolation` that do not
    /// depend on a variable being set, with Vector's answers.
    #[test]
    fn the_syntax_is_vectors() {
        assert_eq!(yaml("$$FOO"), "$FOO");
        assert_eq!(yaml("$ x"), "$ x");
        assert_eq!(yaml("${FOO x"), "${FOO x");
        assert_eq!(yaml("${}"), "${}");
        assert_eq!(yaml("${:-cats}"), "${:-cats}");
        assert_eq!(yaml("${NOT:-dogcats}"), "dogcats");
        assert_eq!(yaml("${NOT:-dogs and cats}"), "dogs and cats");
        assert_eq!(yaml("${NOT:-}"), "");
        assert_eq!(yaml("${NOT-cats}"), "cats");
    }

    /// Unset is all this can assume, and an unset variable with no default is
    /// not something Vector would start with; its written form is the most
    /// honest thing to show.
    #[test]
    fn a_variable_without_a_default_keeps_its_written_form() {
        assert_eq!(yaml("path: ${LOG_DIR}/app.log"), "path: ${LOG_DIR}/app.log");
        assert_eq!(yaml("x: $FOO.BAR"), "x: $FOO.BAR");
        assert_eq!(toml("path = \"${LOG_DIR}/a\""), "path = \"${LOG_DIR}/a\"");
        assert_eq!(toml("x = \"${A:?required}\""), "x = \"${A:?required}\"");
        assert_eq!(toml("# ${A}\n"), "# ${A}\n");
    }

    #[test]
    fn a_bare_toml_value_is_quoted_so_it_parses() {
        assert_eq!(
            toml("max_size = ${BUFFER_SIZE_BYTES}\n"),
            "max_size = \"${BUFFER_SIZE_BYTES}\"\n"
        );
        assert_eq!(toml("port = $PORT"), "port = \"${PORT}\"");
        assert_eq!(toml("x = ${A:?nope}"), "x = \"${A}\"");
        assert_eq!(toml("x = ${A:-1024}"), "x = 1024");
        assert_eq!(toml("s = '''\n${A}\n'''\nn = ${B}"), "s = '''\n${A}\n'''\nn = \"${B}\"");
        assert_eq!(toml("s = \"a\\\"${A}\""), "s = \"a\\\"${A}\"");
    }

    #[test]
    fn offsets_map_back_to_the_file_as_written() {
        let source = "a = ${LONG_NAME:-1}\nb = 2";
        let read = interpolate(source, Syntax::Toml);
        assert_eq!(read.text, "a = 1\nb = 2");

        let b = read.text.find('b').unwrap();
        assert_eq!(&source[read.original(b)..], "b = 2");
        assert_eq!(read.original(4), 4, "the start of a replacement is the variable's");
        assert_eq!(read.original(5), source.find('\n').unwrap(), "its end is the variable's end");
        assert_eq!(read.original(2), 2, "nothing before an edit moves");
    }

    #[test]
    fn quoting_moves_offsets_too() {
        let source = "x = ${A}\ny = 1";
        let read = interpolate(source, Syntax::Toml);
        let y = read.text.find('y').unwrap();
        assert_eq!(read.original(y), source.find('y').unwrap());
    }
}
