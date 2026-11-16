//! Selector bucketing and the per-epoch match cache (§6.2).
//!
//! Two structures, each with a job:
//!
//! - [`RightmostBuckets`] partitions a stylesheet's selectors by the *cheapest
//!   discriminating feature of their rightmost compound*: an id, a class, a
//!   tag, or "universal". A restyle then tests only the buckets an element can
//!   possibly fall into, instead of every rule in the sheet. Built once per
//!   stylesheet, not per restyle.
//! - [`SelectorCache`] remembers `(selector, node) -> bool` for the duration of
//!   one restyle epoch. It is cleared wholesale when the DOM changes, because
//!   any structural mutation can change a structural pseudo result.
//!
//! The buckets are derived from what the CSS parser actually emits, so an
//! unrecognised compound falls into `universal` and is still tested — a miss
//! here would be a silent wrong style, which is the one failure mode we refuse.

use ahash::AHashMap;
use engine_core::NodeId;
use engine_css::selectors::{PseudoClass, Selector, SelectorList, SimpleSelector};

use crate::index::ElementIndex;

/// The bucket a selector belongs to, keyed by its rightmost compound.
#[derive(Clone, Debug, Default)]
pub struct RightmostBuckets {
    /// Selectors with an `#id` in the rightmost compound.
    pub by_id: AHashMap<String, Vec<u32>>,
    /// Selectors with a `.class` in the rightmost compound.
    pub by_class: AHashMap<String, Vec<u32>>,
    /// Selectors with only a type selector (no id/class/attr) in the rightmost.
    pub by_tag: AHashMap<String, Vec<u32>>,
    /// Everything else, including `*`, attribute-only, pseudo-only, and
    /// unsupported compounds. Always tested.
    pub universal: Vec<u32>,
    /// Index into the sheet's selector list, in source order.
    pub total: usize,
}

impl RightmostBuckets {
    /// Bucket every selector in `list`, which is a *concatenation* of rules: the
    /// indices recorded are positions in `list.selectors`, so the caller can map
    /// back with `rule_of_selector`.
    pub fn build(list: &[&SelectorList]) -> RightmostBuckets {
        let mut b = RightmostBuckets::default();
        let mut idx = 0u32;
        for sel_list in list {
            for sel in &sel_list.selectors {
                b.insert(idx, sel);
                idx += 1;
            }
        }
        b.total = idx as usize;
        b
    }

    fn insert(&mut self, idx: u32, sel: &Selector) {
        let Some(c) = sel.compounds.first() else {
            self.universal.push(idx);
            return;
        };
        let mut id = None;
        let mut class = None;
        let mut tag = None;
        for s in &c.simples {
            match s {
                SimpleSelector::Id(v) => id = Some(v.clone()),
                SimpleSelector::Class(v) => {
                    if class.is_none() {
                        class = Some(v.clone());
                    }
                }
                SimpleSelector::Type(t) => {
                    if let Some(n) = &t.name {
                        tag = Some(n.clone());
                    }
                }
                SimpleSelector::Attribute(_) => {
                    // Attribute-only compounds stay universal: building a
                    // per-attribute map would need one bucket per name/value.
                }
                SimpleSelector::PseudoClass(_) | SimpleSelector::PseudoElement(_) => {}
            }
        }
        // Precedence: id beats class beats tag, matching how selective each is.
        if let Some(v) = id {
            self.by_id.entry(v).or_default().push(idx);
        } else if let Some(v) = class {
            self.by_class.entry(v).or_default().push(idx);
        } else if let Some(v) = tag {
            self.by_tag.entry(v).or_default().push(idx);
        } else {
            self.universal.push(idx);
        }
    }

    /// Push every bucket index that could conceivably match `el` into `out`.
    /// The result is not deduplicated and not ordered; callers only test it.
    pub fn candidates(&self, el: &ElementIndex, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.universal);
        if let Some(id) = &el.id {
            if let Some(v) = self.by_id.get(id.as_ref()) {
                out.extend_from_slice(v);
            }
        }
        for c in &el.classes {
            if let Some(v) = self.by_class.get(c.as_ref()) {
                out.extend_from_slice(v);
            }
        }
        if let Some(t) = &el.tag {
            if let Some(v) = self.by_tag.get(t.as_str()) {
                out.extend_from_slice(v);
            }
        }
    }
}

/// A bounded direct-mapped cache for `(selector, node)` match results.
///
/// Direct-mapped (not a hash map) because the keys are already dense integers
/// and cache lookups on the hot path should not hash or allocate. Collisions
/// evict, so the worst case is a recompute.
pub struct SelectorCache {
    slots: Vec<u64>,
    len: u64,
}

impl SelectorCache {
    pub fn new(bits: u32) -> Self {
        let n = 1usize << bits;
        SelectorCache { slots: vec![u64::MAX; n], len: n as u64 }
    }

    #[inline]
    fn key(sel: u32, node: NodeId) -> u64 {
        ((sel as u64) << 32) | node.0 as u64
    }

    #[inline]
    fn slot(&self, key: u64) -> usize {
        // A multiplicative mix; low bits of the key are node ids that vary by 1.
        let h = key.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        ((h >> 32) as usize) & (self.len as usize - 1)
    }

    pub fn get(&self, sel: u32, node: NodeId) -> Option<bool> {
        let k = Self::key(sel, node);
        let s = self.slots[self.slot(k)];
        if s == u64::MAX {
            None
        } else if s == k {
            Some(true)
        } else if s == k | (1 << 63) {
            Some(false)
        } else {
            None
        }
    }

    pub fn put(&mut self, sel: u32, node: NodeId, matched: bool) {
        let k = Self::key(sel, node) | if matched { 0 } else { 1 << 63 };
        let i = self.slot(Self::key(sel, node));
        self.slots[i] = k;
    }

    /// Invalidate everything. Called when the DOM changes structure.
    pub fn clear(&mut self) {
        self.slots.fill(u64::MAX);
    }
}

/// True if a selector needs a live DOM and therefore cannot be cached across
/// epochs (§6.4: `:has()` excluded from the cache). Structural pseudo-classes
/// are cacheable *within* an epoch, which is what this cache is for.
pub fn is_structural(sel: &Selector) -> bool {
    fn has_structural(c: &engine_css::CompoundSelector) -> bool {
        c.simples.iter().any(|s| {
            matches!(
                s,
                SimpleSelector::PseudoClass(
                    PseudoClass::FirstChild
                        | PseudoClass::LastChild
                        | PseudoClass::OnlyChild
                        | PseudoClass::NthChild { .. }
                        | PseudoClass::NthLastChild { .. }
                        | PseudoClass::FirstOfType
                        | PseudoClass::LastOfType
                        | PseudoClass::OnlyOfType
                        | PseudoClass::Empty
                )
            )
        })
    }
    sel.compounds.iter().any(has_structural)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_css::{parse_selector_list, tokenize};

    fn sel(src: &str) -> Selector {
        parse_list(src).selectors.remove(0)
    }

    fn parse_list(src: &str) -> SelectorList {
        parse_selector_list(&tokenize(src)).expect("valid selector")
    }

    #[test]
    fn buckets_by_rightmost_id_class_tag() {
        let (a, b, c, d) = ("#main .x", "p.text", "div p", "*");
        let buckets = RightmostBuckets::build(&[&parse_list(a), &parse_list(b), &parse_list(c), &parse_list(d)]);
        // `#main .x` ends in `.x`, `p.text` ends in `p.text` (class wins),
        // `div p` ends in `p`, `*` is universal.
        assert_eq!(buckets.by_class.get("x"), Some(&vec![0u32]));
        assert_eq!(buckets.by_class.get("text"), Some(&vec![1u32]));
        assert_eq!(buckets.by_tag.get("p"), Some(&vec![2u32]));
        assert_eq!(buckets.universal, vec![3u32]);
        assert_eq!(buckets.total, 4);
    }

    #[test]
    fn universal_always_candidate() {
        let buckets = RightmostBuckets::build(&[&parse_list("*"), &parse_list("p")]);
        let mut out = Vec::new();
        let el = ElementIndex { tag: Some(engine_dom::Tag::lookup("span")), ..Default::default() };
        buckets.candidates(&el, &mut out);
        assert_eq!(out, vec![0u32], "only the universal bucket for a span");
    }

    #[test]
    fn structural_detection() {
        assert!(is_structural(&sel("p:nth-child(2)")));
        assert!(!is_structural(&sel("p.x")));
    }
}