# Topology corpus

Synthetic Vector configurations for the topology graph. Public config format
only: no third-party pipelines, no sample logs, and the VRL kept to the
smallest thing that compiles, because what is under test here is `inputs`, not
the transforms themselves.

Each file exercises one shape. The ones under `broken/` are wrong on purpose,
one finding each, and a check that stops reporting one of them is a regression.

There is no `vector` binary in this repository, so these are written against
the published configuration reference. If you have Vector installed,
`vector validate --no-environment` over the valid ones is the honest
cross-check and is worth running after touching them.
