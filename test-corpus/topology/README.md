# Topology corpus

Synthetic Vector configurations for the topology graph. Public config format
only: no third-party pipelines, no sample logs, and the VRL kept to the
smallest thing that compiles, because what is under test here is `inputs`, not
the transforms themselves.

Each file exercises one shape. The ones under `broken/` are wrong on purpose,
one finding each, and a check that stops reporting one of them is a regression.

Several of them exist for one rule each of the outputs table:
`ports.yaml` for the two sources that have named outputs, `enrichment.yaml`
for the `memory` table that is a sink and a source at once,
`relaxed-wildcards.toml` for the global that makes a pattern matching nothing
legal.

There is no `vector` binary in this repository, so these are written against
Vector's own source at the pinned release —
`crates/vector-topology/tests/against_vector.rs` re-reads the port names from
it. If you have Vector installed, `vector validate --no-environment` over the
valid ones is the honest cross-check and is worth running after touching them.
