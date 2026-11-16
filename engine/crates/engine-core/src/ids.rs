//! Arena identity types. One newtype per arena so a `NodeId` can never be
//! passed where a `BoxId` is expected (§3.2: arenas, not `Rc`).

use std::fmt;

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub const INVALID: $name = $name(u32::MAX);

            pub const fn new(v: u32) -> Self {
                $name(v)
            }

            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

id_type!(NodeId, "Index into `engine_dom::Document`'s node arena.");
id_type!(StyleId, "Index into the computed-style arena.");
id_type!(BoxId, "Index into the box tree arena.");
id_type!(FontId, "Index into the font context's face arena.");
id_type!(FragmentId, "Index into the layout fragment arena.");
id_type!(ImageKey, "Index into the image cache.");
// Index into the compiled selector arena; used as the selector-cache key (§6.2).
id_type!(SelectorId, "Index into the compiled selector arena.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_types() {
        // Compile-time check documented as a test: constructing one id type from
        // another's value requires an explicit `.0`, so mixups are caught.
        let n = NodeId::new(3);
        let b = BoxId::new(3);
        assert_eq!(n.0, b.0);
        assert_eq!(n.index(), 3);
        assert_eq!(format!("{n}"), "NodeId(3)");
    }

    #[test]
    fn invalid_is_max() {
        assert_eq!(NodeId::INVALID.0, u32::MAX);
    }
}