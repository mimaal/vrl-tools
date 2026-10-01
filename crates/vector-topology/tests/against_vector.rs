//! Re-reads `src/outputs.rs`'s claims from Vector's own source.
//!
//! The outputs table is the one thing in this crate that cannot be generated:
//! knowing that an `opentelemetry` source has a `logs` port means compiling
//! Vector, which is not something a test suite for an editor extension is
//! going to do. So it is written down — and checked here against the source it
//! was written down from, whenever that source happens to be on the machine.
//!
//! It is there more often than not. The workspace takes `vector-vrl-functions`
//! as a git dependency on Vector at the pinned tag, so building anything in
//! this repo leaves a full checkout under `CARGO_HOME/git/checkouts`, CI
//! included. When it is not there — a fresh clone, an offline machine — the
//! test skips rather than fails: it is a check on a claim, not a build
//! dependency.
//!
//! What it checks is the *names*. A port being renamed or dropped is both the
//! likeliest way for the table to go stale and the one with the worst
//! symptom — an input that resolves to nothing, on a config Vector runs
//! happily. Whether a component still has a default output is prose in
//! `outputs.rs`, next to the function it comes from.

use std::path::{Path, PathBuf};

use vector_topology::outputs::{DROPPED, EXPIRED, LLMOBS, LOGS, METRICS, TRACES, UNMATCHED};

/// The tag the workspace pins Vector at, read from the manifest so the two
/// cannot drift.
fn pinned_tag() -> String {
    let manifest = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
    )
    .expect("the workspace manifest reads");

    let tag = manifest
        .split("tag = \"v")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the workspace pins Vector at a tag");
    tag.to_owned()
}

/// A checkout of Vector at the pinned release, if cargo has one.
///
/// The directory under `checkouts` is named for a hash of the repository URL
/// and its subdirectories for commits, so neither can be predicted; the
/// version in the manifest is what says a checkout is the right one.
fn vector_source() -> Option<PathBuf> {
    let home = std::env::var_os("CARGO_HOME").map_or_else(
        || {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".cargo"))
        },
        |home| Some(PathBuf::from(home)),
    )?;

    let wanted = format!("version = \"{}\"", pinned_tag());
    let repositories = std::fs::read_dir(home.join("git/checkouts")).ok()?;
    for repository in repositories.flatten() {
        if !repository.file_name().to_string_lossy().starts_with("vector-") {
            continue;
        }
        for commit in std::fs::read_dir(repository.path()).ok()?.flatten() {
            let root = commit.path();
            let manifest = match std::fs::read_to_string(root.join("Cargo.toml")) {
                Ok(manifest) => manifest,
                Err(_) => continue,
            };
            if manifest.lines().any(|line| line.trim() == wanted) {
                return Some(root);
            }
        }
    }
    None
}

/// Every port name this crate claims, and the declaration in Vector it was
/// read from.
fn claims() -> Vec<(&'static str, &'static str, String)> {
    vec![
        (
            "src/transforms/route.rs",
            UNMATCHED,
            format!("const UNMATCHED_ROUTE: &str = \"{UNMATCHED}\";"),
        ),
        (
            "src/transforms/exclusive_route/config.rs",
            UNMATCHED,
            "UNMATCHED_ROUTE".to_owned(),
        ),
        (
            "src/transforms/remap.rs",
            DROPPED,
            format!("const DROPPED: &str = \"{DROPPED}\";"),
        ),
        (
            "src/enrichment_tables/memory/source.rs",
            EXPIRED,
            format!("const EXPIRED_ROUTE: &str = \"{EXPIRED}\";"),
        ),
        (
            "src/sources/opentelemetry/config.rs",
            LOGS,
            format!("pub const LOGS: &str = \"{LOGS}\";"),
        ),
        (
            "src/sources/opentelemetry/config.rs",
            METRICS,
            format!("pub const METRICS: &str = \"{METRICS}\";"),
        ),
        (
            "src/sources/opentelemetry/config.rs",
            TRACES,
            format!("pub const TRACES: &str = \"{TRACES}\";"),
        ),
        (
            "src/sources/datadog_agent/mod.rs",
            LOGS,
            format!("pub const LOGS: &str = \"{LOGS}\";"),
        ),
        (
            "src/sources/datadog_agent/mod.rs",
            METRICS,
            format!("pub const METRICS: &str = \"{METRICS}\";"),
        ),
        (
            "src/sources/datadog_agent/mod.rs",
            TRACES,
            format!("pub const TRACES: &str = \"{TRACES}\";"),
        ),
        (
            "src/sources/datadog_agent/mod.rs",
            LLMOBS,
            format!("pub const LLMOBS: &str = \"{LLMOBS}\";"),
        ),
    ]
}

#[test]
fn every_port_name_is_still_the_one_vector_declares() {
    let Some(root) = vector_source() else {
        eprintln!(
            "skipped: no checkout of Vector v{} under CARGO_HOME/git/checkouts",
            pinned_tag(),
        );
        return;
    };

    for (file, port, declaration) in claims() {
        let source = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|error| panic!("{file}: {error}"));
        assert!(
            source.contains(&declaration),
            "`{port}` is claimed as an output of {file}, but Vector v{} no longer declares it \
             ({declaration}). Re-read `fn outputs` there and update crates/vector-topology/src/outputs.rs.",
            pinned_tag(),
        );
    }
}

/// The two flag names the table reads to decide a `datadog_agent`'s and a
/// `route`'s outputs. A renamed flag is read as absent, which silently brings
/// back the wrong shape.
#[test]
fn every_flag_the_table_reads_still_exists() {
    let Some(root) = vector_source() else {
        return;
    };

    for (file, flags) in [
        (
            "src/sources/datadog_agent/mod.rs",
            &[
                "multiple_outputs",
                "disable_logs",
                "disable_metrics",
                "disable_traces",
                "disable_llmobs",
            ][..],
        ),
        ("src/transforms/route.rs", &["reroute_unmatched"][..]),
        ("src/transforms/remap.rs", &["reroute_dropped"][..]),
        (
            "src/enrichment_tables/memory/config.rs",
            &["source_key", "export_expired_items"][..],
        ),
    ] {
        let source = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|error| panic!("{file}: {error}"));
        for flag in flags {
            assert!(
                source.contains(&format!("{flag}:")),
                "crates/vector-topology/src/outputs.rs reads `{flag}`, which {file} no longer has",
            );
        }
    }
}

/// `reroute_dropped` is `remap`'s alone, which is why the table keys it on the
/// type. If another transform grows one, the table has to grow with it.
#[test]
fn reroute_dropped_is_still_only_remaps() {
    let Some(root) = vector_source() else {
        return;
    };

    let transforms = std::fs::read_dir(root.join("src/transforms")).expect("transforms read");
    for entry in transforms.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let source = if path.is_dir() {
            std::fs::read_to_string(path.join("config.rs")).unwrap_or_default()
        } else {
            std::fs::read_to_string(&path).unwrap_or_default()
        };

        if name.starts_with("remap") {
            continue;
        }
        assert!(
            !source.contains("pub reroute_dropped") && !source.contains("    reroute_dropped:"),
            "{name} has grown a `reroute_dropped`, so it has a `dropped` output that \
             crates/vector-topology/src/outputs.rs does not know about",
        );
    }
}

/// The merge `config::assemble` reproduces: the top-level files of a
/// `--config-dir` read as one value before any component is built, with
/// `merge_values`' rules — lists concatenated, a scalar replaced by the later
/// one of its kind, two kinds refused.
///
/// And the one place the crate knowingly differs: it merges in file-name
/// order, where Vector merges in the order `read_dir` lists the directory —
/// its `serde_json` keeps insertion order, and nothing sorts. If Vector starts
/// sorting, the difference is gone and the doc on `config::merge` is wrong.
#[test]
fn the_merge_is_still_vectors() {
    let Some(root) = vector_source() else {
        return;
    };
    let read = |file: &str| {
        std::fs::read_to_string(root.join(file)).unwrap_or_else(|error| panic!("{file}: {error}"))
    };

    let representation = read("src/config/loading/representation.rs");
    for rule in [
        "(Value::String(_), Value::String(other)) => Ok(Value::String(other))",
        "(Value::Bool(_), Value::Bool(other)) => Ok(Value::Bool(other))",
        "if number_type(&value) == number_type(&other)",
        "value.extend(other);",
        "Incompatible types at path",
    ] {
        assert!(
            representation.contains(rule),
            "merge_values no longer has `{rule}`; re-read it and update Value's merge in \
             crates/vector-topology/src/config.rs",
        );
    }

    let loader = read("src/config/loading/loader.rs");
    assert!(
        loader.contains("merge_into_map(&mut root, map)?"),
        "load_from_dir no longer merges the top-level files into one value",
    );
    let load_dir_into = loader
        .split("fn load_dir_into")
        .nth(1)
        .and_then(|rest| rest.split("fn load_file").next())
        .expect("load_dir_into is still there");
    assert!(
        !load_dir_into.contains("sort"),
        "Vector now sorts a config directory; config::merge's doc says it does not",
    );

    let manifest = read("Cargo.toml");
    assert!(
        manifest
            .lines()
            .any(|line| line.starts_with("serde_json") && line.contains("preserve_order")),
        "Vector's serde_json no longer keeps insertion order",
    );
}

/// What `src/lookups.rs` and `src/tables.rs` take for granted about how a
/// table is read: the two functions, the table as their first parameter and a
/// compile-time constant, the three places a `remap` keeps its program, where
/// a table's data file is written, and that Vector warns about the outputs of
/// sources and transforms and nothing else — so an unread table is no warning.
#[test]
fn the_lookups_are_still_how_a_table_is_read() {
    let Some(root) = vector_source() else {
        return;
    };
    let read = |file: &str| {
        std::fs::read_to_string(root.join(file)).unwrap_or_else(|error| panic!("{file}: {error}"))
    };

    for function in vector_topology::lookups::FUNCTIONS {
        let source = read(&format!("lib/vector-vrl/enrichment/src/{function}.rs"));
        assert!(source.contains(&format!("\"{function}\"")), "{function} was renamed");
        assert!(
            source.contains(".required_enum(\"table\", &tables, state)"),
            "{function} no longer takes its table as a constant among the declared ones",
        );
        let parameters = source
            .split("const PARAMETERS: &[Parameter] = &[")
            .nth(1)
            .expect("the parameters are still declared");
        assert!(
            parameters.trim_start().starts_with("Parameter::required(\n        \"table\","),
            "`table` is no longer the first parameter of {function}",
        );
    }

    let remap = read("src/transforms/remap.rs");
    for field in [
        "pub source: Option<String>",
        "pub file: Option<PathBuf>",
        "pub files: Option<Vec<PathBuf>>",
    ] {
        assert!(remap.contains(field), "a remap no longer has `{field}`");
    }

    for (file, field) in [
        ("src/enrichment_tables/file.rs", "pub path: PathBuf"),
        ("src/enrichment_tables/geoip.rs", "pub path: PathBuf"),
        ("src/enrichment_tables/mmdb.rs", "pub path: PathBuf"),
    ] {
        assert!(read(file).contains(field), "{file} no longer has `{field}`");
    }
    assert!(
        read("src/enrichment_tables/file.rs").contains("pub file: FileSettings"),
        "a file table's path is no longer under `file`",
    );

    assert!(
        read("src/config/validation.rs")
            .contains("for (input_type, id) in transform_ids.chain(source_ids)"),
        "validation::warnings walks something other than source and transform outputs; \
         re-read it before saying an unread table is not a warning",
    );
    assert!(
        read("src/conditions/mod.rs").contains("vrl_config.build(enrichment_tables, metrics_storage)"),
        "a string condition is no longer VRL compiled against the tables",
    );
}
