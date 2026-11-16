//! Right-to-left selector matching.
//!
//! Entry points:
//! - [`matches_selector`] — one complex selector against one node.
//! - [`matches_compound`] — one compound against one node (used by tests and by
//!   the `:not()`/`:is()`/`:where()`/`:has()` evaluators).
//!
//! Design (§6.1): matching starts at the candidate element with the rightmost
//! compound — which `engine-css` stores as `compounds[0]` — and walks leftwards
//! across combinators. The walk is an explicit stack, not recursion, so a
//! selector with 10k compounds cannot blow the stack.
//!
//! **Termination.** Each loop visits a strictly shorter prefix of the
//! ancestor/sibling chain and stops at `None`, and the explicit stack only grows
//! with compounds from a finite selector. A selector loaded from a hostile
//! stylesheet therefore terminates. The only recursion is
//! `:not()`/`:is()`/`:where()`/`:has()` nesting, bounded by
//! [`MAX_PSEUDO_DEPTH`]; beyond it a nested predicate reports "no match" — a
//! miss, never a hang.

use engine_core::NodeId;
use engine_css::selectors::{
    AttrOp, AttributeSelector, Combinator, CompoundSelector, PseudoClass, Selector, SelectorList,
    SimpleSelector, TypeSelector,
};
use engine_dom::{Document, KnownTag, Tag};

use crate::index::{ElementIndex, MatchIndex};

/// Maximum nesting of pseudo-class arguments we evaluate (`:not(:not(...))`).
/// `engine-css` caps the *parse* at 64; matching keeps its own cap because a
/// selector can also be built programmatically.
pub const MAX_PSEUDO_DEPTH: u32 = 64;

/// `:has()` search depth into the subtree (§6.4: depth-2 descendant search).
pub const MAX_HAS_DEPTH: u32 = 2;

/// Ceiling on any ancestor/sibling walk. Depth is bounded by the DOM itself, so
/// this is defensive only: it converts a (structurally impossible) cycle in the
/// link fields into a miss instead of a hang.
const MAX_CHAIN: u32 = 1 << 20;

/// State pseudo-classes that depend on input, not on the tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MatchFlags {
    pub hovered: Option<NodeId>,
    pub focused: Option<NodeId>,
    pub active: Option<NodeId>,
}

impl MatchFlags {
    pub fn is_hovered(&self, id: NodeId) -> bool {
        self.hovered == Some(id)
    }
}

/// Matching context: the document, the per-element index, and input flags.
pub struct MatchCtx<'a> {
    pub doc: &'a Document,
    pub index: &'a MatchIndex,
    pub flags: MatchFlags,
}

impl<'a> MatchCtx<'a> {
    pub fn new(doc: &'a Document, index: &'a MatchIndex) -> Self {
        MatchCtx { doc, index, flags: MatchFlags::default() }
    }

    pub fn with_flags(doc: &'a Document, index: &'a MatchIndex, flags: MatchFlags) -> Self {
        MatchCtx { doc, index, flags }
    }
}

/// Does the complex `sel` match element `subject`?
pub fn matches_selector(sel: &Selector, subject: NodeId, ctx: &MatchCtx<'_>) -> bool {
    let mut stack = Vec::new();
    matches_selector_inner(sel, subject, ctx, 0, &mut stack)
}

fn matches_selector_inner(
    sel: &Selector,
    subject: NodeId,
    ctx: &MatchCtx<'_>,
    depth: u32,
    stack: &mut Vec<(usize, NodeId)>,
) -> bool {
    if depth > MAX_PSEUDO_DEPTH {
        return false;
    }
    let compounds = sel.compounds.as_slice();
    // Fast reject on the rightmost compound: one tag/hash compare instead of a
    // whole ancestor walk. This is the filter that makes a restyle linear.
    if compounds.is_empty() || !matches_compound_inner(&compounds[0], subject, ctx, depth, stack) {
        return false;
    }
    if compounds.len() == 1 {
        return true;
    }
    push_step(stack, sel, 0, subject, ctx);
    // `compounds` is stored subject-first, so the walk is finished when the
    // *last* compound (the leftmost in source order) has matched.
    let leftmost = compounds.len() - 1;
    while let Some((idx, node)) = stack.pop() {
        if !matches_compound_inner(&compounds[idx], node, ctx, depth, stack) {
            continue;
        }
        if idx == leftmost {
            return true;
        }
        push_step(stack, sel, idx, node, ctx);
    }
    false
}

/// Push the next candidate(s) for `compounds[idx + 1]` after `compounds[idx]`
/// matched at `node`. Explicit stack replaces the recursion in §6.1's sketch.
fn push_step(
    stack: &mut Vec<(usize, NodeId)>,
    sel: &Selector,
    idx: usize,
    node: NodeId,
    ctx: &MatchCtx<'_>,
) {
    let next = idx + 1;
    // `idx` is always `< len - 1` here: the caller returns once the leftmost
    // compound has matched. Keeping the guard preserves that invariant if the
    // caller ever changes.
    if next >= sel.compounds.len() {
        return;
    }
    // `compounds` and `combinators` are both stored right-to-left, so
    // `combinators[idx]` relates `compounds[idx]` (right) to `compounds[idx+1]`.
    match sel.combinators[idx] {
        Combinator::Child => {
            if let Some(p) = ctx.doc.parent(node) {
                if ctx.index.get(p).is_some() {
                    stack.push((next, p));
                }
            }
        }
        Combinator::Descendant => {
            // Collect farthest-first so the LIFO pop tries the nearest ancestor.
            let mut ancestors = Vec::new();
            let mut cur = ctx.doc.parent(node);
            let mut steps = 0u32;
            while let Some(a) = cur {
                steps += 1;
                if steps > MAX_CHAIN {
                    break;
                }
                if ctx.index.get(a).is_some() {
                    ancestors.push(a);
                }
                cur = ctx.doc.parent(a);
            }
            stack.extend(ancestors.into_iter().rev().map(|a| (next, a)));
        }
        Combinator::AdjacentSibling => {
            if let Some(p) = prev_element_sibling(ctx.doc, node) {
                stack.push((next, p));
            }
        }
        Combinator::GeneralSibling => {
            let mut sibs = Vec::new();
            let mut cur = prev_element_sibling(ctx.doc, node);
            let mut steps = 0u32;
            while let Some(s) = cur {
                steps += 1;
                if steps > MAX_CHAIN {
                    break;
                }
                sibs.push(s);
                cur = prev_element_sibling(ctx.doc, s);
            }
            stack.extend(sibs.into_iter().rev().map(|s| (next, s)));
        }
        // `||` column combinator is unsupported (§15): never matches.
        Combinator::Column => {}
    }
}

fn prev_element_sibling(doc: &Document, node: NodeId) -> Option<NodeId> {
    let mut cur = doc.node(node).prev_sibling;
    let mut steps = 0u32;
    while let Some(s) = cur {
        steps += 1;
        if steps > MAX_CHAIN {
            return None;
        }
        if doc.element(s).is_some() {
            return Some(s);
        }
        cur = doc.node(s).prev_sibling;
    }
    None
}

/// Does one compound match `node`?
pub fn matches_compound(c: &CompoundSelector, node: NodeId, ctx: &MatchCtx<'_>) -> bool {
    let mut stack = Vec::new();
    matches_compound_inner(c, node, ctx, 0, &mut stack)
}

fn matches_compound_inner(
    c: &CompoundSelector,
    node: NodeId,
    ctx: &MatchCtx<'_>,
    depth: u32,
    stack: &mut Vec<(usize, NodeId)>,
) -> bool {
    if depth > MAX_PSEUDO_DEPTH {
        return false;
    }
    let Some(el) = ctx.index.get(node) else { return false };
    for s in &c.simples {
        let ok = match s {
            SimpleSelector::Type(t) => match_type(t, el),
            SimpleSelector::Id(id) => el.id.as_deref() == Some(id.as_str()),
            SimpleSelector::Class(cls) => el.classes.contains(cls.as_str()),
            SimpleSelector::Attribute(a) => match_attribute(a, ctx.doc, node),
            SimpleSelector::PseudoClass(p) => match_pseudo(p, node, el, ctx, depth, stack),
            // Pseudo-elements are not generated in v1 (§15), so a selector
            // that requires one matches no real element.
            SimpleSelector::PseudoElement(_) => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

fn match_type(t: &TypeSelector, el: &ElementIndex) -> bool {
    match &t.name {
        None => true, // `*`
        Some(name) => el.has_tag(name),
    }
}

fn match_attribute(a: &AttributeSelector, doc: &Document, node: NodeId) -> bool {
    let Some(e) = doc.element(node) else { return false };
    let Some(v) = e.attr(&a.name) else { return false };
    let pat = a.value.as_deref().unwrap_or("");
    if a.case_insensitive {
        let v = v.to_ascii_lowercase();
        let pat = pat.to_ascii_lowercase();
        attr_op_matches(a.op, &v, &pat)
    } else {
        attr_op_matches(a.op, v, pat)
    }
}

fn attr_op_matches(op: AttrOp, v: &str, pat: &str) -> bool {
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => v == pat,
        AttrOp::Includes => v.split_ascii_whitespace().any(|w| w == pat),
        AttrOp::DashMatch => v == pat || (v.starts_with(pat) && v.as_bytes().get(pat.len()) == Some(&b'-')),
        AttrOp::Prefix => !pat.is_empty() && v.starts_with(pat),
        AttrOp::Suffix => !pat.is_empty() && v.ends_with(pat),
        AttrOp::Substring => !pat.is_empty() && v.contains(pat),
    }
}

fn match_pseudo(
    p: &PseudoClass,
    node: NodeId,
    el: &ElementIndex,
    ctx: &MatchCtx<'_>,
    depth: u32,
    stack: &mut Vec<(usize, NodeId)>,
) -> bool {
    match p {
        PseudoClass::FirstChild => el.child_index == 1,
        PseudoClass::LastChild => el.child_index == el.sibling_count,
        PseudoClass::OnlyChild => el.sibling_count == 1,
        PseudoClass::FirstOfType => el.type_index == 1,
        PseudoClass::LastOfType => el.type_index == el.type_count,
        PseudoClass::OnlyOfType => el.type_count == 1,
        PseudoClass::Root => ctx.doc.parent(node) == Some(ctx.doc.root()),
        PseudoClass::Empty => el.empty,
        PseudoClass::Hover => ctx.flags.is_hovered(node),
        PseudoClass::Focus | PseudoClass::FocusVisible | PseudoClass::FocusWithin => {
            ctx.flags.focused == Some(node)
        }
        PseudoClass::Active => ctx.flags.active == Some(node),
        PseudoClass::Visited => false,
        PseudoClass::Link => matches!(doc_tag(ctx.doc, node), Some(t) if t.is(KnownTag::A)),
        PseudoClass::Checked => is_checked(ctx.doc, node),
        PseudoClass::Disabled => is_disabled(ctx.doc, node),
        PseudoClass::Enabled => is_enabled(ctx.doc, node),
        PseudoClass::NthChild { a, b, of } => {
            let n = match of {
                None => el.child_index,
                Some(inner) => {
                    // §15: `:nth-child(An+B of S)` where S is not a single
                    // compound never matches.
                    if inner.compounds.len() != 1 {
                        return false;
                    }
                    let mut n = 0u32;
                    for sib in element_siblings(ctx.doc, node) {
                        if matches_compound_inner(&inner.compounds[0], sib, ctx, depth + 1, stack) {
                            n += 1;
                        }
                    }
                    if !matches_nth(n, *a, *b) {
                        return false;
                    }
                    n
                }
            };
            matches_nth(n, *a, *b)
        }
        PseudoClass::NthLastChild { a, b } => {
            let n = el.sibling_count + 1 - el.child_index;
            matches_nth(n, *a, *b)
        }
        PseudoClass::NthOfType { a, b } => matches_nth(el.type_index, *a, *b),
        PseudoClass::Not(list) => !list_matches(list, node, ctx, depth + 1, stack),
        PseudoClass::Is(list) | PseudoClass::Where(list) => {
            list_matches(list, node, ctx, depth + 1, stack)
        }
        PseudoClass::Has(list) => has_matches(list, node, ctx, depth + 1, stack),
        PseudoClass::Unsupported(_) => false,
    }
}

fn doc_tag(doc: &Document, node: NodeId) -> Option<&Tag> {
    doc.element(node).map(|e| &e.tag)
}

fn element_siblings(doc: &Document, node: NodeId) -> Vec<NodeId> {
    let Some(parent) = doc.parent(node) else { return Vec::new() };
    let mut out = Vec::new();
    let mut cur = doc.first_child(parent);
    let mut steps = 0u32;
    while let Some(c) = cur {
        steps += 1;
        if steps > MAX_CHAIN {
            break;
        }
        if doc.element(c).is_some() {
            out.push(c);
        }
        cur = doc.next_sibling(c);
    }
    out
}

/// `An+B` test against a 1-based index.
///
/// `n` may be any integer ≥ 0, so with a positive `A` the smallest match is
/// `B` (clamped to ≥ 1) and every `A`-th index after it.
fn matches_nth(index: u32, a: i32, b: i32) -> bool {
    let idx = index as i64;
    let (a, b) = (a as i64, b as i64);
    if a == 0 {
        return idx == b;
    }
    let diff = idx - b;
    diff % a == 0 && diff / a >= 0
}

fn list_matches(
    list: &SelectorList,
    node: NodeId,
    ctx: &MatchCtx<'_>,
    depth: u32,
    stack: &mut Vec<(usize, NodeId)>,
) -> bool {
    list.selectors
        .iter()
        .any(|s| matches_selector_inner(s, node, ctx, depth, stack))
}

/// `:has()`: depth-limited descendant search (§6.4). `engine-css` does not
/// model relative selectors (a leading `>`/`+`/`~` inside `:has()`), so only
/// descendant relationships are searched.
fn has_matches(
    list: &SelectorList,
    node: NodeId,
    ctx: &MatchCtx<'_>,
    depth: u32,
    stack: &mut Vec<(usize, NodeId)>,
) -> bool {
    if depth > MAX_PSEUDO_DEPTH {
        return false;
    }
    let mut level = element_children(ctx.doc, node);
    for _ in 0..MAX_HAS_DEPTH {
        let mut next_level = Vec::new();
        for &cand in &level {
            if list
                .selectors
                .iter()
                .any(|s| matches_selector_inner(s, cand, ctx, depth, stack))
            {
                return true;
            }
            next_level.extend(element_children(ctx.doc, cand));
        }
        if next_level.is_empty() {
            return false;
        }
        level = next_level;
    }
    false
}

fn element_children(doc: &Document, node: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut cur = doc.first_child(node);
    let mut steps = 0u32;
    while let Some(c) = cur {
        steps += 1;
        if steps > MAX_CHAIN {
            break;
        }
        if doc.element(c).is_some() {
            out.push(c);
        }
        cur = doc.next_sibling(c);
    }
    out
}

fn is_checked(doc: &Document, node: NodeId) -> bool {
    let Some(e) = doc.element(node) else { return false };
    if !(e.tag.is(KnownTag::Input) || e.tag.is(KnownTag::Option)) {
        return false;
    }
    e.has_attr("checked") || e.has_attr("selected")
}

fn is_disabled(doc: &Document, node: NodeId) -> bool {
    let Some(e) = doc.element(node) else { return false };
    let form_control = matches!(
        e.tag.as_known(),
        Some(KnownTag::Input | KnownTag::Button | KnownTag::Select | KnownTag::Textarea | KnownTag::Option)
    );
    if !form_control {
        return false;
    }
    if e.has_attr("disabled") {
        return true;
    }
    // Disabled by an ancestor `<fieldset disabled>` (HTML §4.10.4).
    let mut cur = doc.parent(node);
    let mut steps = 0u32;
    while let Some(p) = cur {
        steps += 1;
        if steps > MAX_CHAIN {
            break;
        }
        if let Some(pe) = doc.element(p) {
            if pe.tag.is(KnownTag::Fieldset) && pe.has_attr("disabled") {
                return true;
            }
        }
        cur = doc.parent(p);
    }
    false
}

fn is_enabled(doc: &Document, node: NodeId) -> bool {
    let Some(e) = doc.element(node) else { return false };
    let form_control = matches!(
        e.tag.as_known(),
        Some(KnownTag::Input | KnownTag::Button | KnownTag::Select | KnownTag::Textarea | KnownTag::Option)
    );
    form_control && !is_disabled(doc, node)
}