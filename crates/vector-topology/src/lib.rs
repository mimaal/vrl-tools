//! Where events go in a Vector configuration.
//!
//! Reads a `vector.yaml` or `vector.toml` and works out the path an event
//! takes: which source produces it, which transforms it crosses, which sink it
//! ends in. Knows nothing about WebAssembly or VS Code, like the rest of
//! `crates/`.
//!
//! The graph is not inferred. Every transform and sink declares `inputs`, so
//! the edges are written down in the file. The work is resolving what they
//! name, because an input can be a wildcard, and it can name one particular
//! output of a component rather than the component itself.

pub mod config;
pub mod outputs;

pub use config::{read_toml, read_yaml, Component, ConfigError, Input, Role};
