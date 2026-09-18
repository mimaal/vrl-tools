//! The outputs a component offers besides its default one.
//!
//! This is the one place in the crate that knows Vector's component semantics
//! rather than just its file format, so it is small on purpose and every rule
//! in it comes from the published configuration reference:
//!
//! - a `route` transform has one output per route, named after the route, and
//!   additionally `_unmatched` unless `reroute_unmatched` is `false`;
//! - a transform with `reroute_dropped` has a `dropped` output.
//!
//! A consumer names one with a dot: `split.errors`, `split._unmatched`,
//! `strict.dropped`. That is what makes an edge a port rather than a node, and
//! it is the difference between "this transform feeds that sink" and "the
//! events that failed to parse feed that sink".
//!
//! When a component type grows an output that is not listed here, the effect
//! is an input nobody can resolve, reported rather than silently dropped. That
//! is the intended failure: this table is a claim about Vector, and claims
//! about someone else's software go stale.

/// The output Vector gives every component, which an input names by bare ID.
pub const DEFAULT: &str = "_default";

/// The output a `route` sends events matching no route to.
pub const UNMATCHED: &str = "_unmatched";

/// The output a transform sends events it dropped to.
pub const DROPPED: &str = "dropped";

/// The named outputs a component declares, given what its own configuration
/// says.
///
/// `routes` is `Some` only for a component that has a `route` map at all,
/// which is what tells a route transform from everything else without reading
/// the `type` field — a config can name a type this crate has never heard of.
#[must_use]
pub fn declared(
    routes: Option<Vec<String>>,
    reroute_dropped: bool,
    reroute_unmatched: bool,
) -> Vec<String> {
    let mut outputs = Vec::new();

    if let Some(routes) = routes {
        outputs.extend(routes);
        if reroute_unmatched {
            outputs.push(UNMATCHED.to_owned());
        }
    }

    if reroute_dropped {
        outputs.push(DROPPED.to_owned());
    }

    outputs
}

#[cfg(test)]
mod tests {
    use super::{declared, DROPPED, UNMATCHED};

    #[test]
    fn a_plain_transform_declares_nothing() {
        assert!(declared(None, false, true).is_empty());
    }

    #[test]
    fn a_route_declares_its_routes_and_unmatched() {
        let routes = Some(vec!["errors".to_owned(), "warnings".to_owned()]);
        assert_eq!(
            declared(routes, false, true),
            ["errors", "warnings", UNMATCHED],
        );
    }

    /// `reroute_unmatched: false` does not merely stop events reaching the
    /// output, it removes the output. An input naming it is then wrong, and
    /// saying so is the point.
    #[test]
    fn unmatched_can_be_turned_off() {
        let routes = Some(vec!["errors".to_owned()]);
        assert_eq!(declared(routes, false, false), ["errors"]);
    }

    #[test]
    fn dropped_is_independent_of_routing() {
        assert_eq!(declared(None, true, true), [DROPPED]);
    }
}
