//! What Vector adds on top of the standard library.
//!
//! A `remap` transform does not compile against `vrl::stdlib::all()`. It
//! compiles against `vector_vrl_functions::all()`, which is the standard
//! library plus the functions only Vector has: the enrichment table lookups,
//! the secrets, `set_semantic_meaning`. Checking a program against the
//! standard library alone reports `find_enrichment_table_records` as an
//! undefined function, which is a false positive in every pipeline that
//! enriches.
//!
//! The functions are Vector's own, taken from the Vector release the `vrl` pin
//! comes from, not reimplemented. Two groups are left out on purpose: the
//! metrics functions need `vector-core` and a live metrics store, and
//! `parse_dnstap` needs the dnstap protobuf parser. Neither belongs in the
//! module the extension ships.
//!
//! The enrichment functions are the ones with a condition attached. Their
//! `compile` asks for the `TableRegistry` Vector builds from the config's
//! `enrichment_tables`, and rejects a table name that is not in it — at
//! compile time, the same way `vector validate` does. So a program can only be
//! checked against the tables of the config it runs in, and the caller has to
//! say which those are.

use std::collections::HashMap;

use enrichment::{Case, Condition, Error, IndexHandle, Table, TableRegistry};
use vector_vrl_functions::set_semantic_meaning::MeaningList;
use vrl::compiler::CompileConfig;
use vrl::value::{ObjectMap, Value};

use crate::Diagnostic;

/// The compile configuration a `remap` transform uses, as Vector sets it up in
/// `RemapConfig::compile_vrl_program`: the enrichment tables, and the list
/// `set_semantic_meaning` records into.
///
/// Vector also registers its metrics storage there, which only the metrics
/// functions read, and those are not compiled in.
pub(crate) fn compile_config(enrichment_tables: &[String]) -> CompileConfig {
    let registry = TableRegistry::default();
    registry.load(
        enrichment_tables
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    Box::new(Declared {
                        name: name.clone(),
                        indexes: Vec::new(),
                    }) as Box<dyn Table + Send + Sync>,
                )
            })
            .collect::<HashMap<_, _>>(),
    );

    let mut config = CompileConfig::default();
    config.set_custom(registry);
    config.set_custom(MeaningList::default());
    config
}

/// Moves the tables from loading to reading, as Vector does once every
/// transform has compiled. Until then a lookup fails with "finish_load not
/// called", which would be this crate's bug on the user's screen.
pub(crate) fn finish_loading(config: &CompileConfig) {
    if let Some(registry) = config.get_custom::<TableRegistry>() {
        registry.finish_load();
    }
}

/// A table the config declares, without its data.
///
/// What the compiler checks against the registry is the table's name. It then
/// asks the table to index the fields the condition searches, and a real table
/// can refuse: a `geoip` table only searches by `ip`, a `file` table only by
/// its columns. Answering that needs the data — the database, the CSV — which
/// lives on the machine Vector runs on. So this table accepts every index, and
/// that is the one question about an enrichment call this checker leaves to
/// Vector. It errs towards saying nothing rather than towards a false error.
///
/// Nothing here holds rows either, so a program that looks one up while
/// running in the editor stops with Vector's own "table not loaded" error,
/// which is the truth: the data is not loaded here.
#[derive(Clone)]
struct Declared {
    name: String,
    indexes: Vec<(Case, Vec<String>)>,
}

impl Declared {
    fn not_loaded(&self) -> Error {
        Error::TableNotLoaded {
            table: self.name.clone(),
        }
    }
}

impl Table for Declared {
    fn find_table_row<'a>(
        &self,
        _case: Case,
        _condition: &'a [Condition<'a>],
        _select: Option<&[String]>,
        _wildcard: Option<&Value>,
        _index: Option<IndexHandle>,
    ) -> Result<ObjectMap, Error> {
        Err(self.not_loaded())
    }

    fn find_table_rows<'a>(
        &self,
        _case: Case,
        _condition: &'a [Condition<'a>],
        _select: Option<&[String]>,
        _wildcard: Option<&Value>,
        _index: Option<IndexHandle>,
    ) -> Result<Vec<ObjectMap>, Error> {
        Err(self.not_loaded())
    }

    fn add_index(&mut self, case: Case, fields: &[&str]) -> Result<IndexHandle, Error> {
        self.indexes
            .push((case, fields.iter().map(|&field| field.to_owned()).collect()));
        Ok(IndexHandle(self.indexes.len() - 1))
    }

    fn index_fields(&self) -> Vec<(Case, Vec<String>)> {
        self.indexes.clone()
    }

    fn needs_reload(&self) -> bool {
        false
    }
}

/// What an enrichment call naming a table the registry does not have comes
/// back as. The function's own error is an invalid enum variant; the compiler
/// reports it as a failure to compile that call, keeping its message and
/// labels.
const FUNCTION_COMPILATION: usize =
    vrl::compiler::codes::CompilerCode::FunctionCompilation as usize;

/// Says where table names come from, on the diagnostic that rejects one, when
/// no config declared any.
///
/// The compiler's own labels list the tables it would have accepted. With none
/// declared that list is empty, and "expected one of: " followed by nothing
/// reads like a bug in the checker rather than a config it could not find.
pub(crate) fn explain_missing_tables(diagnostics: &mut [Diagnostic], enrichment_tables: &[String]) {
    if !enrichment_tables.is_empty() {
        return;
    }

    for diagnostic in diagnostics {
        let rejects_the_table = diagnostic.labels.iter().any(|label| {
            label.primary && label.message == r#"invalid enum variant for argument "table""#
        });
        if diagnostic.code == FUNCTION_COMPILATION && rejects_the_table {
            diagnostic.notes.push(
                "no Vector config in this workspace declares `enrichment_tables`; \
                 table names are checked against the ones a config declares"
                    .to_owned(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{check, is_error, run, Diagnostic};

    const LOOKUP: &str = r#".owner = get_enrichment_table_record!("hosts", {"host": .host})"#;

    fn errors(source: &str, tables: &[&str]) -> Vec<Diagnostic> {
        let tables: Vec<String> = tables.iter().map(|&t| t.to_owned()).collect();
        check(source, None, &tables)
            .diagnostics
            .into_iter()
            .filter(is_error)
            .collect()
    }

    /// The false positive this module exists for. Before it, both of these were
    /// "call to undefined function".
    #[test]
    fn vectors_own_functions_are_defined() {
        for source in [
            LOOKUP,
            r#".rows = find_enrichment_table_records!("hosts", {"host": .host})"#,
            r#".key = get_secret("api_key")"#,
            r#"set_secret("api_key", "hunter2")"#,
            r#"remove_secret("api_key")"#,
            r#"set_semantic_meaning(.message, "message")"#,
        ] {
            let errors = errors(source, &["hosts"]);
            assert!(errors.is_empty(), "{source}: {errors:?}");
        }
    }

    /// The table name is checked at compile time, as `vector validate` checks
    /// it, and a misspelt one is pointed at.
    #[test]
    fn a_table_the_config_does_not_declare_is_an_error() {
        let errors = errors(LOOKUP, &["hostnames"]);
        let error = errors.first().expect("an undeclared table is rejected");

        assert_eq!(error.code, super::FUNCTION_COMPILATION);
        assert!(
            error
                .labels
                .iter()
                .any(|label| label.message.contains("hostnames")),
            "the compiler lists the tables it would accept: {error:?}",
        );
        assert!(error.notes.iter().all(|note| !note.contains("workspace")));
    }

    #[test]
    fn with_no_tables_declared_the_error_says_where_they_come_from() {
        let errors = errors(LOOKUP, &[]);
        let error = errors.first().expect("no table can be looked up");

        assert!(
            error
                .notes
                .iter()
                .any(|note| note.contains("enrichment_tables")),
            "{error:?}",
        );
    }

    /// The metrics functions are left out on purpose; this is the test that
    /// notices if they arrive by accident, which would mean `vector-core` has.
    #[test]
    fn the_metrics_functions_are_not_compiled_in() {
        let errors = errors(r#".m = get_vector_metric("uptime")"#, &[]);

        assert!(errors.iter().any(|e| e.code == 105), "{errors:?}");
    }

    /// Running has to agree with checking, and a lookup has no rows to find
    /// here. It stops with Vector's own error, not with one from this crate.
    #[test]
    fn running_a_lookup_reports_the_table_as_not_loaded() {
        let source = r#".owner = get_enrichment_table_record("hosts", {"host": .host}) ?? null"#;
        let result = run(source, r#"{"host":"web-01"}"#, &["hosts".to_owned()]);

        assert!(result.compiled, "{:?}", result.diagnostics);
        assert_eq!(result.error, None);
        assert_eq!(
            result.event,
            Some(serde_json::json!({"host": "web-01", "owner": null}))
        );

        let result = run(
            &source
                .replace(" ?? null", "")
                .replace("record(", "record!("),
            r#"{"host":"web-01"}"#,
            &["hosts".to_owned()],
        );
        let error = result.error.expect("the lookup fails");
        assert!(error.contains("Table hosts not loaded"), "{error}");
    }
}
