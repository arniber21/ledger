//! Untrusted generated Protobuf storage.
//!
//! Values in this module have only wire-level validity. Convert them through
//! the validated domain types before making verification decisions.

/// Generated canonical model messages.
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod model {
    /// Version 1 of the canonical model wire contract.
    pub mod v1 {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../gen/rust/sentinel.model.v1.rs"
        ));
    }
}

/// Generated Probe protocol messages and service descriptors.
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod probe {
    /// Version 1 of the Probe decision messages and schema descriptor metadata.
    pub mod v1 {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../gen/rust/sentinel.probe.v1.rs"
        ));

        /// Canonical descriptor set retaining `SentinelDecisionService.Decide` metadata.
        ///
        /// Transport clients and servers remain owned by the language SDK issues.
        pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../gen/rust/sentinel.file_descriptor_set.binpb"
        ));
    }
}
