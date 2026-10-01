//! Which enrichment tables the VRL in a config looks up.
//!
//! An enrichment table is not wired into the pipeline: nothing lists it in
//! `inputs`. It is reached from VRL, by name, with
//! `find_enrichment_table_records` and `get_enrichment_table_record`
//! (`lib/vector-vrl/enrichment/src/`). So who reads a table is a question
//! about program text, and this is the reader of that text.
//!
//! It is a reader, not a compiler, and it answers one thing: which table
//! names are written as the `table` argument of those two calls. It never
//! says whether a program is valid — that is the real compiler's, in
//! `vrl-check-core`, and this crate does not link it. What makes the reading
//! safe is that it only looks for a function name followed by `(`, outside
//! comments and strings; there is no other construct in VRL that looks like
//! that.
//!
//! Both functions take the table with `required_enum("table", &tables, state)`
//! (`find_enrichment_table_records.rs`, `get_enrichment_table_record.rs`), so
//! the argument has to be a compile-time constant — but not a literal:
//! `name = "hosts"` followed by `get_enrichment_table_record!(name, …)`
//! validates, as `vector validate` 0.58.0 confirmed. A call like that is
//! reported as [`Found::opaque`] rather than guessed at, so "nothing reads this
//! table" is never said about a pipeline where something might.
//!
//! The VRL is wherever Vector compiles VRL against the tables: a `remap`'s
//! `source`, the programs its `file` and `files` name (`src/transforms/remap.rs`
//! — "if a relative path is provided, its root is the current working
//! directory"), and every condition, which is VRL when it is a string or says
//! `type: vrl` (`src/conditions/mod.rs`, `AnyCondition::build`, which is handed
//! the tables too). Rather than list the fields conditions can hide in, every
//! string in a component is read: only a program can contain one of these
//! calls.

/// The two lookups, as they are called.
pub const FUNCTIONS: [&str; 2] = [
    "find_enrichment_table_records",
    "get_enrichment_table_record",
];

/// What a piece of VRL looks up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// The tables named by a string literal, in the order first met, once
    /// each.
    pub tables: Vec<String>,
    /// Whether some call names its table any other way — a variable, a
    /// template — which the compiler can follow and this cannot.
    pub opaque: bool,
}

impl Found {
    pub(crate) fn merge(&mut self, other: Self) {
        for table in other.tables {
            if !self.tables.contains(&table) {
                self.tables.push(table);
            }
        }
        self.opaque |= other.opaque;
    }
}

/// Reads the enrichment lookups out of a VRL program.
#[must_use]
pub fn scan(program: &str) -> Found {
    let bytes = program.as_bytes();
    let mut found = Found::default();
    let mut at = 0;

    while at < bytes.len() {
        match bytes[at] {
            b'#' => at = line_end(bytes, at),
            b'"' => at = string_end(bytes, at),
            // `s'…'`, `r'…'`, `t'…'`: raw strings, regexes and timestamps. A
            // quote that follows anything else is not VRL, and is stepped
            // over as a string all the same so its contents are not read.
            b'\'' => at = raw_end(bytes, at),
            byte if is_ident(byte) => {
                let end = ident_end(bytes, at);
                let word = &program[at..end];
                at = end;
                if FUNCTIONS.contains(&word) {
                    if let Some(open) = call_open(bytes, end) {
                        match table_argument(program, open + 1) {
                            Some(table) => {
                                if !found.tables.contains(&table) {
                                    found.tables.push(table);
                                }
                            }
                            None => found.opaque = true,
                        }
                    }
                }
            }
            _ => at += 1,
        }
    }

    found
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn ident_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < bytes.len() && is_ident(bytes[at]) {
        at += 1;
    }
    at
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(bytes.len(), |offset| from + offset)
}

/// Past the closing quote of the `"…"` string opening at `from`.
fn string_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

/// Past the closing quote of the `'…'` literal opening at `from`.
fn raw_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'\'' => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

fn skip_space(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < bytes.len() {
        match bytes[at] {
            byte if byte.is_ascii_whitespace() => at += 1,
            b'#' => at = line_end(bytes, at),
            _ => break,
        }
    }
    at
}

/// The `(` of a call whose name ends at `from`, past an optional `!`.
fn call_open(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    if bytes.get(at) == Some(&b'!') {
        at += 1;
    }
    let at = skip_space(bytes, at);
    (bytes.get(at) == Some(&b'(')).then_some(at)
}

/// The table a call names, when it names it with a string literal: the first
/// argument, or the one written `table: …` wherever it comes.
fn table_argument(program: &str, from: usize) -> Option<String> {
    let bytes = program.as_bytes();
    let first = skip_space(bytes, from);

    // Positional: the table is the first parameter of both functions.
    if bytes.get(first) == Some(&b'"') {
        return literal(program, first);
    }

    // By keyword, anywhere among the arguments of this call and no deeper.
    let mut depth = 0usize;
    let mut at = first;
    while at < bytes.len() {
        match bytes[at] {
            b'#' => at = line_end(bytes, at),
            b'"' => at = string_end(bytes, at),
            b'\'' => at = raw_end(bytes, at),
            b'(' | b'[' | b'{' => {
                depth += 1;
                at += 1;
            }
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                at += 1;
            }
            byte if is_ident(byte) => {
                let end = ident_end(bytes, at);
                if depth == 0 && &program[at..end] == "table" {
                    let colon = skip_space(bytes, end);
                    if bytes.get(colon) == Some(&b':') {
                        let value = skip_space(bytes, colon + 1);
                        return (bytes.get(value) == Some(&b'"'))
                            .then(|| literal(program, value))
                            .flatten();
                    }
                }
                at = end;
            }
            _ => at += 1,
        }
    }
    None
}

/// The contents of the `"…"` literal opening at `from`.
///
/// `None` for one with an escape or a `{{ template }}` in it: a table is
/// named with neither, and unescaping here would be a second implementation
/// of the compiler's.
fn literal(program: &str, from: usize) -> Option<String> {
    let end = string_end(program.as_bytes(), from);
    let inner = program.get(from + 1..end.checked_sub(1)?)?;
    (!inner.contains('\\') && !inner.contains("{{")).then(|| inner.to_owned())
}

#[cfg(test)]
mod tests {
    use super::scan;

    fn tables(program: &str) -> Vec<String> {
        scan(program).tables
    }

    #[test]
    fn a_literal_first_argument_is_the_table() {
        assert_eq!(
            tables(r#".a = get_enrichment_table_record("hosts", { "host": .host })"#),
            ["hosts"],
        );
        assert_eq!(
            tables(r#".a = find_enrichment_table_records!("geo", { "net": .net })"#),
            ["geo"],
        );
    }

    #[test]
    fn the_keyword_form_is_read_wherever_it_comes() {
        assert_eq!(
            tables(r#"find_enrichment_table_records!(table: "geo", condition: { "a": .a })"#),
            ["geo"],
        );
        assert_eq!(
            tables(
                "get_enrichment_table_record!(\n  condition: { \"table\": .a },\n  table: \"hosts\"\n)"
            ),
            ["hosts"],
        );
    }

    #[test]
    fn each_table_is_listed_once_in_the_order_met() {
        assert_eq!(
            tables(
                "a = get_enrichment_table_record!(\"b\", {})\n\
                 c = get_enrichment_table_record!(\"a\", {})\n\
                 d = find_enrichment_table_records!(\"b\", {})\n"
            ),
            ["b", "a"],
        );
    }

    #[test]
    fn a_call_in_a_comment_or_a_string_is_not_a_call() {
        let found = scan(
            "# get_enrichment_table_record(\"commented\", {})\n\
             .note = \"get_enrichment_table_record(\\\"quoted\\\", {})\"\n\
             .raw = s'find_enrichment_table_records(\"raw\", {})'\n",
        );
        assert!(found.tables.is_empty(), "{found:?}");
        assert!(!found.opaque);
    }

    /// The name has to be the whole identifier, and it has to be called.
    #[test]
    fn a_longer_name_or_a_bare_mention_is_not_a_call() {
        let found = scan(
            "my_get_enrichment_table_record(\"x\", {})\n\
             get_enrichment_table_record_v2(\"y\", {})\n\
             .f = get_enrichment_table_record\n",
        );
        assert!(found.tables.is_empty(), "{found:?}");
        assert!(!found.opaque);
    }

    /// `vector validate` accepts a constant held in a variable, so the call
    /// reads *some* table. Which one is the compiler's to say.
    #[test]
    fn a_table_named_any_other_way_is_opaque_not_guessed() {
        let found = scan("name = \"hosts\"\n.a = get_enrichment_table_record!(name, {})\n");
        assert!(found.tables.is_empty());
        assert!(found.opaque);

        assert!(scan(r#"get_enrichment_table_record!("{{ kind }}_hosts", {})"#).opaque);
    }

    #[test]
    fn a_program_cut_short_does_not_panic() {
        for program in [
            "get_enrichment_table_record",
            "get_enrichment_table_record!(",
            "get_enrichment_table_record!(\"",
            "get_enrichment_table_record!(table",
            "get_enrichment_table_record!(table:",
            "\"unterminated \\",
            "s'",
        ] {
            let _ = scan(program);
        }
    }
}
