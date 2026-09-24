//! The outputs a component offers.
//!
//! This is the one place in the crate that knows Vector's component semantics
//! rather than just its file format, so it is small on purpose, and every rule
//! in it is checked against the `outputs()` of the component in Vector's
//! source at the release the `vrl` pin comes from, `v<VECTOR_RELEASE>`.
//!
//! What an input may name is `id` when the component has a default output and
//! `id.port` for each named one — exactly the strings Vector builds in
//! `expand_globs` and looks up in `Graph::input_map` (`src/config/`). A
//! consumer naming a named output with a dot is what makes an edge a port
//! rather than a node, and it is the difference between "this transform feeds
//! that sink" and "the events that failed to parse feed that sink".
//!
//! The rules are keyed on `type`, because that is what decides them in Vector
//! and because nothing else can: an `opentelemetry` source is told from a
//! `file` source by its type and by nothing in its own fields. They are
//! **additive**, though: a type this table has never heard of keeps the
//! default output and no named ones, which is what almost every component
//! has, so a config using a component newer than this table draws as it
//! always did instead of dissolving.
//!
//! The table, with the line of Vector it comes from:
//!
//! **Sources**
//! - `opentelemetry` has `logs`, `metrics` and `traces`, and **no default
//!   output** — an input has to name a port (`src/sources/opentelemetry/config.rs`,
//!   `fn outputs`).
//! - `datadog_agent` with `multiple_outputs: true` has `logs`, `metrics`,
//!   `traces` and `llmobs`, minus each one its `disable_*` flag turns off, and
//!   no default output. Without `multiple_outputs` it has only the default one
//!   (`src/sources/datadog_agent/mod.rs`, `fn outputs`).
//!
//! **Transforms**
//! - `route` has one output per entry of its `route` map, named after it, plus
//!   `_unmatched` unless `reroute_unmatched` is `false` — and no default output
//!   (`src/transforms/route.rs`).
//! - `exclusive_route` has one output per entry of its `routes` list, named by
//!   the entry's `name`, plus `_unmatched` always — and no default output
//!   either; it has no `reroute_unmatched` to turn that off
//!   (`src/transforms/exclusive_route/config.rs`).
//! - `remap` with `reroute_dropped: true` has a `dropped` output besides its
//!   default one. `reroute_dropped` belongs to `remap` alone, so the same key
//!   on any other type means nothing (`src/transforms/remap.rs`).
//!
//! **Enrichment tables**
//! - A table takes `inputs` and offers nothing: Vector compiles it into a sink
//!   (`EnrichmentTableOuter::as_sink`). Only a `memory` table actually accepts
//!   them; the others have no `sink_config`.
//! - A `memory` table with `source_config` is **also a source**, under the
//!   *different* name its `source_key` gives, with the default output and an
//!   `expired` one when `export_expired_items` is set. A table with a `filter`
//!   cannot be a source at all (`src/enrichment_tables/memory/config.rs`,
//!   `fn source_config` and `fn outputs`).
//!
//! When a component type grows an output that is not listed here, the effect
//! is an input nobody can resolve, reported rather than silently dropped. That
//! is the intended failure: this table is a claim about Vector, and claims
//! about someone else's software go stale. `tests/against_vector.rs` re-reads
//! the claim from Vector's own source when it is on the machine.

use crate::config::Role;

/// The output a router sends events matching no route to.
pub const UNMATCHED: &str = "_unmatched";

/// The output a transform sends events it dropped to.
pub const DROPPED: &str = "dropped";

/// The output a `memory` enrichment table flushes expired items to.
pub const EXPIRED: &str = "expired";

/// The ports the two multi-output sources share.
pub const LOGS: &str = "logs";
pub const METRICS: &str = "metrics";
pub const TRACES: &str = "traces";
/// `datadog_agent`'s fourth port.
pub const LLMOBS: &str = "llmobs";

/// What a component offers to the components reading it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outputs {
    /// Whether an input can name the component itself. A router cannot be
    /// read that way, and neither can an `opentelemetry` source: every event
    /// they emit leaves by a named output.
    pub default: bool,
    /// The named outputs, in the order they are declared.
    pub named: Vec<String>,
}

impl Outputs {
    /// Nothing can read this component: a sink, or an enrichment table.
    const fn none() -> Self {
        Self {
            default: false,
            named: Vec::new(),
        }
    }

    /// The default output and nothing else, which is what almost everything
    /// has and what an unknown type is assumed to have.
    const fn plain() -> Self {
        Self {
            default: true,
            named: Vec::new(),
        }
    }

    fn ports(named: Vec<String>) -> Self {
        Self {
            default: false,
            named,
        }
    }
}

/// The fields of one component, as its parser can answer them.
///
/// Both config formats implement this, so the rules above are written once
/// instead of once per parser — which is what let the YAML and the TOML
/// readers disagree about a type before.
pub trait Fields {
    /// A boolean field, or `default` when it is absent or is not a boolean.
    fn flag(&self, key: &str, default: bool) -> bool;

    /// A string field.
    fn text(&self, key: &str) -> Option<String>;

    /// The keys of a mapping field, in the order they are written.
    fn keys(&self, key: &str) -> Option<Vec<String>>;

    /// The `name` of each entry of a list-of-tables field, in order.
    fn names(&self, key: &str) -> Option<Vec<String>>;

    /// Whether a field is there at all, whatever its value.
    fn has(&self, key: &str) -> bool;

    /// A nested table, for the fields that have one.
    fn child(&self, key: &str) -> Option<Self>
    where
        Self: Sized;
}

/// The outputs a component offers, from its role, its `type` and its own
/// fields.
#[must_use]
pub fn declared<F: Fields>(role: Role, component_type: &str, fields: &F) -> Outputs {
    match role {
        // Both are where events stop. An enrichment table is read by VRL, not
        // by another component: Vector compiles it into a sink, and the source
        // half of a `memory` table is a separate component with its own name.
        // See [`table_source`].
        Role::Sink | Role::Table => Outputs::none(),
        Role::Source => source(component_type, fields),
        Role::Transform => transform(component_type, fields),
    }
}

fn source<F: Fields>(component_type: &str, fields: &F) -> Outputs {
    match component_type {
        // Always three ports and no default, whatever the config says.
        "opentelemetry" => Outputs::ports(vec![
            LOGS.to_owned(),
            METRICS.to_owned(),
            TRACES.to_owned(),
        ]),
        "datadog_agent" if fields.flag("multiple_outputs", false) => {
            let mut named = Vec::with_capacity(4);
            for (port, disabled) in [
                (LOGS, "disable_logs"),
                (METRICS, "disable_metrics"),
                (TRACES, "disable_traces"),
                (LLMOBS, "disable_llmobs"),
            ] {
                if !fields.flag(disabled, false) {
                    named.push(port.to_owned());
                }
            }
            Outputs::ports(named)
        }
        _ => Outputs::plain(),
    }
}

fn transform<F: Fields>(component_type: &str, fields: &F) -> Outputs {
    match component_type {
        "route" => {
            let mut named = fields.keys("route").unwrap_or_default();
            if fields.flag("reroute_unmatched", true) {
                named.push(UNMATCHED.to_owned());
            }
            Outputs::ports(named)
        }
        "exclusive_route" => {
            let mut named = fields.names("routes").unwrap_or_default();
            named.push(UNMATCHED.to_owned());
            Outputs::ports(named)
        }
        "remap" if fields.flag("reroute_dropped", false) => Outputs {
            default: true,
            named: vec![DROPPED.to_owned()],
        },
        _ => Outputs::plain(),
    }
}

/// The source half of an enrichment table, when it has one: the name it is
/// read by, and what it offers.
///
/// Only a `memory` table has one, only when `source_config` is there, and not
/// when a `filter` is — a cuckoo or bloom filter cannot be exported. The name
/// is `source_config.source_key`, which Vector requires to differ from the
/// table's own, so this is always a second component rather than another face
/// of the same one.
#[must_use]
pub fn table_source<F: Fields>(component_type: &str, fields: &F) -> Option<(String, Outputs)> {
    if component_type != "memory" || fields.has("filter") {
        return None;
    }

    let source = fields.child("source_config")?;
    let key = source.text("source_key")?;
    let named = if source.flag("export_expired_items", false) {
        vec![EXPIRED.to_owned()]
    } else {
        Vec::new()
    };

    Some((
        key,
        Outputs {
            default: true,
            named,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{declared, table_source, Fields, DROPPED, EXPIRED, UNMATCHED};
    use crate::config::Role;

    /// A hand-built component, so the rules can be tested without a parser.
    #[derive(Default)]
    struct Stub {
        flags: BTreeMap<String, bool>,
        texts: BTreeMap<String, String>,
        keys: BTreeMap<String, Vec<String>>,
        names: BTreeMap<String, Vec<String>>,
        children: BTreeMap<String, Stub>,
        present: Vec<String>,
    }

    impl Stub {
        fn flag(mut self, key: &str, value: bool) -> Self {
            self.flags.insert(key.to_owned(), value);
            self.present.push(key.to_owned());
            self
        }
        fn text(mut self, key: &str, value: &str) -> Self {
            self.texts.insert(key.to_owned(), value.to_owned());
            self.present.push(key.to_owned());
            self
        }
        fn keys(mut self, key: &str, values: &[&str]) -> Self {
            let values = values.iter().map(|v| (*v).to_owned()).collect();
            self.keys.insert(key.to_owned(), values);
            self.present.push(key.to_owned());
            self
        }
        fn names(mut self, key: &str, values: &[&str]) -> Self {
            let values = values.iter().map(|v| (*v).to_owned()).collect();
            self.names.insert(key.to_owned(), values);
            self.present.push(key.to_owned());
            self
        }
        fn child(mut self, key: &str, value: Stub) -> Self {
            self.children.insert(key.to_owned(), value);
            self.present.push(key.to_owned());
            self
        }
        fn bare(mut self, key: &str) -> Self {
            self.present.push(key.to_owned());
            self
        }
    }

    impl Fields for Stub {
        fn flag(&self, key: &str, default: bool) -> bool {
            self.flags.get(key).copied().unwrap_or(default)
        }
        fn text(&self, key: &str) -> Option<String> {
            self.texts.get(key).cloned()
        }
        fn keys(&self, key: &str) -> Option<Vec<String>> {
            self.keys.get(key).cloned()
        }
        fn names(&self, key: &str) -> Option<Vec<String>> {
            self.names.get(key).cloned()
        }
        fn has(&self, key: &str) -> bool {
            self.present.iter().any(|name| name == key)
        }
        fn child(&self, key: &str) -> Option<Self> {
            self.children.get(key).map(|child| Stub {
                flags: child.flags.clone(),
                texts: child.texts.clone(),
                keys: child.keys.clone(),
                names: child.names.clone(),
                children: BTreeMap::new(),
                present: child.present.clone(),
            })
        }
    }

    fn outputs(role: Role, component_type: &str, fields: &Stub) -> (bool, Vec<String>) {
        let outputs = declared(role, component_type, fields);
        (outputs.default, outputs.named)
    }

    #[test]
    fn an_unknown_type_keeps_the_default_output_and_nothing_else() {
        let (default, named) = outputs(Role::Transform, "something_new", &Stub::default());
        assert!(default);
        assert!(named.is_empty());

        let (default, named) = outputs(Role::Source, "file", &Stub::default());
        assert!(default);
        assert!(named.is_empty());
    }

    #[test]
    fn a_sink_and_a_table_offer_nothing() {
        for role in [Role::Sink, Role::Table] {
            let (default, named) = outputs(role, "console", &Stub::default());
            assert!(!default, "{role:?}");
            assert!(named.is_empty(), "{role:?}");
        }
    }

    /// The reason this table had to be keyed on `type`: nothing in an
    /// `opentelemetry` source's own fields says it has ports.
    #[test]
    fn opentelemetry_has_three_ports_and_no_default() {
        let (default, named) = outputs(Role::Source, "opentelemetry", &Stub::default());
        assert!(!default);
        assert_eq!(named, ["logs", "metrics", "traces"]);
    }

    #[test]
    fn datadog_agent_splits_only_when_asked() {
        let (default, named) = outputs(Role::Source, "datadog_agent", &Stub::default());
        assert!(default, "without `multiple_outputs` it is one stream");
        assert!(named.is_empty());

        let split = Stub::default().flag("multiple_outputs", true);
        let (default, named) = outputs(Role::Source, "datadog_agent", &split);
        assert!(!default);
        assert_eq!(named, ["logs", "metrics", "traces", "llmobs"]);
    }

    /// Each `disable_*` removes the port, not just the events reaching it, so
    /// an input naming it afterwards is wrong.
    #[test]
    fn a_disabled_datadog_output_goes_away() {
        let fields = Stub::default()
            .flag("multiple_outputs", true)
            .flag("disable_metrics", true)
            .flag("disable_llmobs", true);
        let (_, named) = outputs(Role::Source, "datadog_agent", &fields);
        assert_eq!(named, ["logs", "traces"]);
    }

    #[test]
    fn a_route_declares_its_routes_and_unmatched_and_no_default() {
        let fields = Stub::default().keys("route", &["errors", "warnings"]);
        let (default, named) = outputs(Role::Transform, "route", &fields);
        assert_eq!(named, ["errors", "warnings", UNMATCHED]);
        assert!(!default);
    }

    /// `reroute_unmatched: false` does not merely stop events reaching the
    /// output, it removes the output. An input naming it is then wrong, and
    /// saying so is the point.
    #[test]
    fn unmatched_can_be_turned_off() {
        let fields = Stub::default()
            .keys("route", &["errors"])
            .flag("reroute_unmatched", false);
        assert_eq!(outputs(Role::Transform, "route", &fields).1, ["errors"]);
    }

    /// `exclusive_route` has no `reroute_unmatched`: its `_unmatched` is
    /// always there.
    #[test]
    fn an_exclusive_route_always_has_unmatched() {
        let fields = Stub::default()
            .names("routes", &["a", "b"])
            .flag("reroute_unmatched", false);
        let (default, named) = outputs(Role::Transform, "exclusive_route", &fields);
        assert_eq!(named, ["a", "b", UNMATCHED]);
        assert!(!default);
    }

    /// `reroute_dropped` is `remap`'s alone. On anything else the key is just
    /// a key, and inventing a `dropped` port for it would be a made-up edge.
    #[test]
    fn dropped_belongs_to_remap() {
        let fields = Stub::default().flag("reroute_dropped", true);
        let (default, named) = outputs(Role::Transform, "remap", &fields);
        assert_eq!(named, [DROPPED]);
        assert!(default);

        assert!(outputs(Role::Transform, "filter", &fields).1.is_empty());
    }

    #[test]
    fn a_memory_table_is_a_source_only_when_it_says_so() {
        assert_eq!(table_source("memory", &Stub::default()), None);
        assert_eq!(
            table_source("file", &Stub::default().child("source_config", Stub::default().text("source_key", "x"))),
            None,
        );

        let table = Stub::default()
            .child("source_config", Stub::default().text("source_key", "cache_out"));
        let (key, outputs) = table_source("memory", &table).expect("a source");
        assert_eq!(key, "cache_out");
        assert!(outputs.default);
        assert!(outputs.named.is_empty());
    }

    #[test]
    fn expired_items_are_a_port_of_their_own() {
        let table = Stub::default().child(
            "source_config",
            Stub::default()
                .text("source_key", "cache_out")
                .flag("export_expired_items", true),
        );
        let (_, outputs) = table_source("memory", &table).expect("a source");
        assert_eq!(outputs.named, [EXPIRED]);
    }

    /// A filtered table holds fingerprints, not rows, so there is nothing to
    /// export and Vector refuses to build the source.
    #[test]
    fn a_filtered_table_cannot_be_a_source() {
        let table = Stub::default()
            .bare("filter")
            .child("source_config", Stub::default().text("source_key", "cache_out"));
        assert_eq!(table_source("memory", &table), None);
    }
}
