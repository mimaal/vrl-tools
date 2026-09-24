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
