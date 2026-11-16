//! The single error type used at every stage boundary (§3.1 `error.rs`).
//!
//! Libraries return this; the binary may convert to `anyhow`-style reporting,
//! but we do not add `anyhow` as a workspace dependency — `Display` on this enum
//! is enough for the CLI's error path.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// A stage was handed input that is structurally impossible (programmer
    /// error surfaced as an error rather than a panic at a library boundary).
    InvalidInput { stage: &'static str, detail: String },
    NotSupported { feature: String },
    /// Resource cap hit (§12: 32 MB body, 8 MB stylesheet, 64 MPix image).
    LimitExceeded { limit: &'static str, actual: u64, max: u64 },
    Io(String),
    Net(String),
    Script(String),
    Layout(String),
}

impl EngineError {
    pub fn invalid(stage: &'static str, detail: impl Into<String>) -> Self {
        EngineError::InvalidInput { stage, detail: detail.into() }
    }

    pub fn unsupported(feature: impl Into<String>) -> Self {
        EngineError::NotSupported { feature: feature.into() }
    }

    pub fn limit(limit: &'static str, actual: u64, max: u64) -> Self {
        EngineError::LimitExceeded { limit, actual, max }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::InvalidInput { stage, detail } => write!(f, "{stage}: {detail}"),
            EngineError::NotSupported { feature } => write!(f, "not supported: {feature}"),
            EngineError::LimitExceeded { limit, actual, max } => {
                write!(f, "limit {limit} exceeded: {actual} > {max}")
            }
            EngineError::Io(e) => write!(f, "io: {e}"),
            EngineError::Net(e) => write!(f, "network: {e}"),
            EngineError::Script(e) => write!(f, "script: {e}"),
            EngineError::Layout(e) => write!(f, "layout: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_informative() {
        let e = EngineError::invalid("html", "unexpected eof in tag");
        assert_eq!(e.to_string(), "html: unexpected eof in tag");
        let e = EngineError::limit("body", 40_000_000, 32_000_000);
        assert_eq!(e.to_string(), "limit body exceeded: 40000000 > 32000000");
    }
}