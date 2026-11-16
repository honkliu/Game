//! Selector AST, parsing, and specificity.

use std::fmt;

/// A type selector, optionally namespaced. `None` tag means `*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeSelector {
    pub name: Option<String>,
}

/// An attribute selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSelector {
    pub name: String,
    pub op: AttrOp,
    pub value: Option<String>,
    pub case_insensitive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Equals,
    Includes,       // ~=
    DashMatch,      // |=
    Prefix,         // ^=
    Suffix,         // $=
    Substring,      // *=
}

/// A structural or state pseudo-class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoClass {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    Root,
    Empty,
    Hover,
    Focus,
    FocusVisible,
    FocusWithin,
    Active,
    Visited,
    Link,
    Checked,
    Disabled,
    Enabled,
    /// `nth-child(An+B [of S])`.
    NthChild {
        a: i32,
        b: i32,
        of: Option<Box<Selector>>,
    },
    NthLastChild {
        a: i32,
        b: i32,
    },
    NthOfType {
        a: i32,
        b: i32,
    },
    Not(Box<SelectorList>),
    Is(Box<SelectorList>),
    Where(Box<SelectorList>),
    Has(Box<SelectorList>),
    /// Unsupported pseudo-classes are kept so the report can name them and the
    /// selector is marked "never matches" rather than silently matching.
    Unsupported(String),
}

/// A simple selector inside a compound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimpleSelector {
    Type(TypeSelector),
    Id(String),
    Class(String),
    Attribute(AttributeSelector),
    PseudoClass(PseudoClass),
    PseudoElement(String),
}

/// A sequence of simple selectors with no combinator between them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    pub simples: Vec<SimpleSelector>,
}

/// The combinator between two compounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
    /// `||`, unsupported (§15).
    Column,
}

/// One complex selector, stored left-to-right (source order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    /// `compounds[0]` is the subject (rightmost) in matching order; this vector
    /// is stored right-to-left so matching walks it forward.
    pub compounds: Vec<CompoundSelector>,
    /// `combinators[i]` relates `compounds[i]` to `compounds[i + 1]`.
    pub combinators: Vec<Combinator>,
    pub specificity: Specificity,
}

/// `(a, b, c)` specificity triple (§5.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity {
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

impl Specificity {
    pub const ZERO: Specificity = Specificity { a: 0, b: 0, c: 0 };

    pub fn new(a: u32, b: u32, c: u32) -> Self {
        Specificity { a, b, c }
    }

    pub fn add(&self, other: Specificity) -> Specificity {
        Specificity {
            a: self.a + other.a,
            b: self.b + other.b,
            c: self.c + other.c,
        }
    }

    /// A packed integer for fast comparison.
    pub fn key(&self) -> u64 {
        ((self.a as u64) << 40) | ((self.b as u64) << 20) | self.c as u64
    }
}

impl fmt::Display for Specificity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{},{})", self.a, self.b, self.c)
    }
}

/// A comma-separated selector list.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SelectorList {
    pub selectors: Vec<Selector>,
}

impl SelectorList {
    pub fn is_empty(&self) -> bool {
        self.selectors.is_empty()
    }
}

// --- Specificity computation ---------------------------------------------

impl CompoundSelector {
    /// Specificity of one compound, before pseudo-class argument handling.
    pub fn specificity(&self) -> Specificity {
        let mut s = Specificity::ZERO;
        for simple in &self.simples {
            s = s.add(simple.specificity());
        }
        s
    }
}

impl SimpleSelector {
    pub fn specificity(&self) -> Specificity {
        match self {
            SimpleSelector::Type(t) => {
                if t.name.is_none() {
                    Specificity::ZERO // `*` contributes nothing
                } else {
                    Specificity::new(0, 0, 1)
                }
            }
            SimpleSelector::Id(_) => Specificity::new(1, 0, 0),
            SimpleSelector::Class(_) => Specificity::new(0, 1, 0),
            SimpleSelector::Attribute(_) => Specificity::new(0, 1, 0),
            SimpleSelector::PseudoElement(_) => Specificity::new(0, 0, 1),
            SimpleSelector::PseudoClass(pc) => pc.specificity(),
        }
    }
}

impl PseudoClass {
    pub fn specificity(&self) -> Specificity {
        match self {
            // `:where()` contributes nothing (§4.2 of Selectors 4).
            PseudoClass::Where(_) => Specificity::ZERO,
            // `:is()`, `:not()`, `:has()` take the max of their arguments.
            PseudoClass::Is(list) | PseudoClass::Not(list) | PseudoClass::Has(list) => {
                list.selectors.iter().map(|s| s.specificity).max().unwrap_or(Specificity::ZERO)
            }
            // `:nth-child(An+B of S)` takes max(its own (0,1,0), S).
            PseudoClass::NthChild { of: Some(inner), .. } => {
                Specificity::new(0, 1, 0).max(inner.specificity)
            }
            // Other pseudo-classes are (0,1,0).
            _ => Specificity::new(0, 1, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(simples: Vec<SimpleSelector>) -> CompoundSelector {
        CompoundSelector { simples }
    }

    #[test]
    fn specificity_of_ids_classes_types() {
        let s = c(vec![
            SimpleSelector::Type(TypeSelector { name: Some("div".into()) }),
            SimpleSelector::Class("x".into()),
            SimpleSelector::Id("y".into()),
        ]);
        assert_eq!(s.specificity(), Specificity::new(1, 1, 1));
    }

    #[test]
    fn universal_selector_contributes_nothing() {
        let s = c(vec![SimpleSelector::Type(TypeSelector { name: None })]);
        assert_eq!(s.specificity(), Specificity::ZERO);
    }

    #[test]
    fn whole_list_takes_max() {
        let inner = SelectorList {
            selectors: vec![
                Selector { compounds: vec![c(vec![SimpleSelector::Class("a".into())])], combinators: vec![], specificity: Specificity::new(0, 1, 0) },
                Selector { compounds: vec![c(vec![SimpleSelector::Id("b".into())])], combinators: vec![], specificity: Specificity::new(1, 0, 0) },
            ],
        };
        assert_eq!(PseudoClass::Is(Box::new(inner)).specificity(), Specificity::new(1, 0, 0));
    }

    #[test]
    fn where_contributes_nothing() {
        let inner = SelectorList {
            selectors: vec![Selector {
                compounds: vec![c(vec![SimpleSelector::Id("b".into())])],
                combinators: vec![],
                specificity: Specificity::new(1, 0, 0),
            }],
        };
        assert_eq!(PseudoClass::Where(Box::new(inner)).specificity(), Specificity::ZERO);
    }

    #[test]
    fn ordering_is_lexicographic() {
        assert!(Specificity::new(0, 2, 0) > Specificity::new(0, 1, 9));
        assert!(Specificity::new(1, 0, 0) > Specificity::new(0, 99, 99));
        assert_eq!(Specificity::new(1, 0, 0).key(), (1u64 << 40));
    }
}