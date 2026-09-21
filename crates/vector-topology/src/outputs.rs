//! The outputs a component offers.
//!
//! This is the one place in the crate that knows Vector's component semantics
//! rather than just its file format, so it is small on purpose, and every rule
//! in it is checked against the `outputs()` of the transform in Vector's
//! source at the release the `vrl` pin comes from:
//!
//! - a `route` transform (`src/transforms/route.rs`) has one output per entry
//!   of its `route` map, named after it, plus `_unmatched` unless
//!   `reroute_unmatched` is `false` — and no default output;
//! - an `exclusive_route` transform (`src/transforms/exclusive_route`) has one
//!   output per entry of its `routes` list, named by the entry's `name`, plus
//!   `_unmatched` always — and no default output either;
//! - a transform with `reroute_dropped` has a `dropped` output besides its
//!   default one;
//! - everything else has just the default output.
//!
//! A consumer names a named output with a dot: `split.errors`,
//! `split._unmatched`, `strict.dropped`. That is what makes an edge a port
//! rather than a node, and it is the difference between "this transform feeds
//! that sink" and "the events that failed to parse feed that sink".
//!
//! When a component type grows an output that is not listed here, the effect
//! is an input nobody can resolve, reported rather than silently dropped. That
//! is the intended failure: this table is a claim about Vector, and claims
//! about someone else's software go stale.

/// The output a router sends events matching no route to.
pub const UNMATCHED: &str = "_unmatched";

/// The output a transform sends events it dropped to.
pub const DROPPED: &str = "dropped";

/// What a component offers to the components reading it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outputs {
    /// Whether an input can name the component itself. A router cannot be
    /// read that way: every event it emits leaves by a named output.
    pub default: bool,
    /// The named outputs, in the order they are declared.
    pub named: Vec<String>,
}

/// The outputs a component declares, given what its own configuration says.
///
/// `routes` is `Some` only for a component with a `route` map, and
/// `exclusive_routes` only for one with a `routes` list: that is what tells
/// the two routers from everything else without reading the `type` field — a
/// config can name a type this crate has never heard of.
#[must_use]
pub fn declared(
    routes: Option<Vec<String>>,
    exclusive_routes: Option<Vec<String>>,
    reroute_dropped: bool,
    reroute_unmatched: bool,
) -> Outputs {
    let mut outputs = Outputs {
        default: true,
        named: Vec::new(),
    };

    if let Some(routes) = routes {
        outputs.default = false;
        outputs.named.extend(routes);
        if reroute_unmatched {
            outputs.named.push(UNMATCHED.to_owned());
        }
    } else if let Some(routes) = exclusive_routes {
        outputs.default = false;
        outputs.named.extend(routes);
        outputs.named.push(UNMATCHED.to_owned());
    }

    if reroute_dropped {
        outputs.named.push(DROPPED.to_owned());
    }

    outputs
}

#[cfg(test)]
mod tests {
    use super::{declared, DROPPED, UNMATCHED};

    #[test]
    fn a_plain_transform_has_only_its_default_output() {
        let outputs = declared(None, None, false, true);
        assert!(outputs.default);
        assert!(outputs.named.is_empty());
    }

    #[test]
    fn a_route_declares_its_routes_and_unmatched_and_no_default() {
        let routes = Some(vec!["errors".to_owned(), "warnings".to_owned()]);
        let outputs = declared(routes, None, false, true);
        assert_eq!(outputs.named, ["errors", "warnings", UNMATCHED]);
        assert!(!outputs.default);
    }

    /// `reroute_unmatched: false` does not merely stop events reaching the
    /// output, it removes the output. An input naming it is then wrong, and
    /// saying so is the point.
    #[test]
    fn unmatched_can_be_turned_off() {
        let routes = Some(vec!["errors".to_owned()]);
        assert_eq!(declared(routes, None, false, false).named, ["errors"]);
    }

    /// `exclusive_route` has no `reroute_unmatched`: its `_unmatched` is
    /// always there.
    #[test]
    fn an_exclusive_route_always_has_unmatched() {
        let routes = Some(vec!["a".to_owned(), "b".to_owned()]);
        let outputs = declared(None, routes, false, false);
        assert_eq!(outputs.named, ["a", "b", UNMATCHED]);
        assert!(!outputs.default);
    }

    #[test]
    fn dropped_is_independent_of_routing() {
        let outputs = declared(None, None, true, true);
        assert_eq!(outputs.named, [DROPPED]);
        assert!(outputs.default);
    }
}
