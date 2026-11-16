//! `engine-select` — right-to-left selector matching against `engine-dom`.
//!
//! The matcher never allocates per compound test except for `:has()` (which
//! collects descendants) and case-insensitive attribute comparisons (which
//! lowercase the value). Both are off the hot path for a typical page.
//!
//! The selector→bitset cache design in ARCHITECTURE.md §6.2 is present in
//! [`cache::SelectorCache`] but is not yet wired into the style pass: a restyle
//! walks every node once and tests each rule's rightmost compound first, which
//! is the same asymptotic cost for a single stylesheet. The rightmost-compound
//! bucketing in [`cache::RightmostBuckets`] is what makes it cheap, and it is
//! wired. Caching only matters on repeated restyles of an unchanged tree, which
//! v1 does not do (every load is a fresh tree).

pub mod cache;
pub mod index;
pub mod matcher;

pub use cache::{RightmostBuckets, SelectorCache};
pub use index::{ElementIndex, MatchIndex};
pub use matcher::{
    matches_compound, matches_selector, MatchCtx, MatchFlags, MAX_HAS_DEPTH, MAX_PSEUDO_DEPTH,
};

/// Which selectors in a sheet can never match, and why.
///
/// `engine-style` uses this to populate `unsupported_selectors` in the render
/// report without re-deriving the answer per element.
pub fn unsupported_selectors(list: &engine_css::SelectorList) -> Vec<String> {
    let mut out = Vec::new();
    for sel in &list.selectors {
        collect_unsupported(sel, &mut out, 0);
    }
    out
}

fn collect_unsupported(sel: &engine_css::Selector, out: &mut Vec<String>, depth: u32) {
    if depth > MAX_PSEUDO_DEPTH {
        out.push("nested selector deeper than the matching limit".into());
        return;
    }
    for c in &sel.compounds {
        for s in &c.simples {
            match s {
                engine_css::SimpleSelector::PseudoClass(p) => match p {
                    engine_css::PseudoClass::Unsupported(name) => out.push(format!(":{name}")),
                    engine_css::PseudoClass::Has(l) => {
                        out.push(":has() (depth-2 descendant search only)".into());
                        for inner in &l.selectors {
                            collect_unsupported(inner, out, depth + 1);
                        }
                    }
                    engine_css::PseudoClass::Not(l)
                    | engine_css::PseudoClass::Is(l)
                    | engine_css::PseudoClass::Where(l) => {
                        for inner in &l.selectors {
                            collect_unsupported(inner, out, depth + 1);
                        }
                    }
                    engine_css::PseudoClass::NthChild { of: Some(inner), .. }
                        if inner.compounds.len() != 1 =>
                    {
                        out.push(":nth-child(An+B of S) with a complex S".into());
                    }
                    _ => {}
                },
                // `engine-css` records a pseudo-element's name with its `::` prefix already
                // applied; do not add another.
                engine_css::SimpleSelector::PseudoElement(name) => out.push(name.clone()),
                _ => {}
            }
        }
    }
    if sel.combinators.contains(&engine_css::Combinator::Column) {
        out.push("|| column combinator".into());
    }
}