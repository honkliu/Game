//! Schema versioning and the incompatibility gate (backlog FND-02).
//!
//! The version is carried **once, on the frame** ([`crate::frame`]), not on every
//! nested payload. One place to check means one place where "reject before
//! interpret" can be enforced; if every nested struct carried its own version,
//! every handler would need its own gate, and a forgotten gate becomes a silently
//! misread payload.

use serde::{Deserialize, Serialize};

use crate::failure::Failure;

/// Major version this binary's consumers understand. A frame whose major differs
/// is rejected before its payload is deserialised.
pub const CONSUMER_SCHEMA_MAJOR: u16 = 1;

/// Highest minor version this binary understands. A higher minor is still
/// accepted: minors are additive-only, and consumers ignore fields they do not
/// know (which `serde` does by default, pinned by a conformance fixture).
pub const CONSUMER_SCHEMA_MINOR: u16 = 0;

/// Version header of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u16,
    pub minor: u16,
}

impl SchemaVersion {
    pub const fn current() -> SchemaVersion {
        SchemaVersion {
            major: CONSUMER_SCHEMA_MAJOR,
            minor: CONSUMER_SCHEMA_MINOR,
        }
    }
}

/// Outcome of the version gate. There is no "accept partially" arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionDecision {
    Accept,
    /// The payload was not interpreted. The caller turns this into an error
    /// response naming the versions; it must not attempt a downgrade parse.
    Reject(Failure),
}

/// Gate a frame's version before any payload decoding.
///
/// Both directions are refused:
/// - newer major: this binary is the older consumer. Interpreting a v2 payload
///   with v1 assumptions is exactly the failure mode FND-02 exists to prevent —
///   a renamed field reads as absent, an absent field reads as false.
/// - older major: a v0 producer talking to a v1 consumer. Refused rather than
///   assumed compatible, because "it happens to still parse" is how a contract
///   break becomes a data bug.
pub fn gate_schema_version(version: &SchemaVersion) -> VersionDecision {
    if version.major > CONSUMER_SCHEMA_MAJOR {
        return VersionDecision::Reject(
            Failure::unsupported("schema.major_unsupported").with_detail(format!(
                "frame major {} exceeds consumer major {}; payload was not interpreted",
                version.major, CONSUMER_SCHEMA_MAJOR
            )),
        );
    }
    if version.major < CONSUMER_SCHEMA_MAJOR {
        return VersionDecision::Reject(
            Failure::unsupported("schema.major_unsupported").with_detail(format!(
                "frame major {} predates consumer major {}; payload was not interpreted",
                version.major, CONSUMER_SCHEMA_MAJOR
            )),
        );
    }
    VersionDecision::Accept
}
