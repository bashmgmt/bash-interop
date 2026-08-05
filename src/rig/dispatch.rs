//! Records calls to bash functions matched by a literal prefix.
//!
//! `Dispatch::new().on(&["DSL", "^"], "DSL")` defines a bash function `DSL`
//! that matches `^` as its first argument, shifts it out, and records
//! `["DSL", …remaining args]`. Prefixes sharing a leading word compile into
//! one function with nested `case` dispatch.

use indexmap::IndexMap;

use super::instrument::{Codegen, Instrument};
use super::src::BashSrc;

const RECORD_ARRAY: &str = "__bc_dispatch_rec";

#[derive(Clone, Debug)]
struct Route {
    matched: Vec<String>,
    tag: String,
}

enum Node {
    Record { tag: String },
    Match { arms: IndexMap<String, Node> },
}

pub struct Dispatch {
    routes: Vec<Route>,
}

impl Default for Dispatch {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatch {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// `matched[0]` is the function name; `matched[1..]` are literal
    /// arguments consumed before the rest is recorded under `tag`.
    pub fn on(mut self, matched: &[&str], tag: &str) -> Self {
        assert!(!matched.is_empty(), "dispatch prefix must name a function");
        self.routes.push(Route {
            matched: matched.iter().map(|word| word.to_string()).collect(),
            tag: tag.to_string(),
        });
        self
    }


    fn functions(&self) -> IndexMap<String, Node> {
        let mut by_function: IndexMap<String, Vec<&Route>> = IndexMap::new();
        for route in &self.routes {
            by_function.entry(route.matched[0].clone()).or_default().push(route);
        }
        by_function.into_iter().map(|(name, routes)| (name, node(&routes, 1))).collect()
    }
}

fn node(routes: &[&Route], depth: usize) -> Node {
    let (terminal, deeper): (Vec<&&Route>, Vec<&&Route>) =
        routes.iter().partition(|route| route.matched.len() == depth);

    assert!(terminal.len() <= 1, "duplicate dispatch prefix at depth {depth}");
    assert!(
        terminal.is_empty() || deeper.is_empty(),
        "dispatch prefix is both terminal and a prefix of another at depth {depth}"
    );

    if let Some(route) = terminal.first() {
        return Node::Record { tag: route.tag.clone() };
    }

    let mut grouped: IndexMap<String, Vec<&Route>> = IndexMap::new();
    for route in deeper {
        grouped.entry(route.matched[depth].clone()).or_default().push(route);
    }
    Node::Match {
        arms: grouped
            .into_iter()
            .map(|(literal, routes)| (literal, node(&routes, depth + 1)))
            .collect(),
    }
}

fn body(node: &Node, depth: usize, codegen: &Codegen) -> BashSrc {
    match node {
        Node::Record { tag } => BashSrc::seq([
            BashSrc::raw(format!(
                "local -a {RECORD_ARRAY}=({} \"$@\")",
                crate::bash::value::emit_scalar(tag)
            )),
            codegen.emit(RECORD_ARRAY),
        ]),
        Node::Match { arms } => {
            let matched = format!("__bc_dispatch_d{depth}");
            BashSrc::seq([
                BashSrc::raw(format!("local {matched}=\"$1\"")),
                BashSrc::raw("shift || return 2"),
                BashSrc::case(
                    &format!("\"${matched}\""),
                    arms.iter().map(|(literal, child)| {
                        (
                            crate::bash::value::emit_scalar(literal),
                            body(child, depth + 1, codegen),
                        )
                    }),
                ),
            ])
        }
    }
}

impl Instrument for Dispatch {
    fn name(&self) -> &str {
        "dispatch"
    }

    fn bash(&self, codegen: &Codegen) -> BashSrc {
        BashSrc::seq(
            self.functions()
                .iter()
                .map(|(name, root)| BashSrc::func(name, body(root, 1, codegen))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render(dispatch: Dispatch) -> String {
        dispatch.bash(&Codegen::new()).as_str().to_string()
    }

    /// A bare prefix records directly; literal arguments become nested case
    /// arms; prefixes sharing a function merge into one definition.
    #[test]
    fn trie_shapes() {
        let bare = render(Dispatch::new().on(&["FOO"], "FOO"));
        assert!(bare.contains("FOO() {"));
        assert!(bare.contains("local -a __bc_dispatch_rec=('FOO' \"$@\")"));
        assert!(!bare.contains("case"));

        let merged =
            render(Dispatch::new().on(&["DSL", "^"], "DSL").on(&["DSL", "%"], "DSL_PCT"));
        assert_eq!(merged.matches("DSL() {").count(), 1);
        assert!(merged.contains("case \"$__bc_dispatch_d1\" in"));
        assert!(merged.contains("'^')") && merged.contains("'%')"));
        assert!(merged.contains("shift || return 2"));

        let nested = render(Dispatch::new().on(&["A", "B", "C"], "X").on(&["A", "B", "D"], "Y"));
        assert_eq!(nested.matches("case").count(), 2);
    }
}
