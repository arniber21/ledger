//! OTLP Adapter and ingest pipeline: normalise telemetry into canonical Events.
//!
//! This crate mirrors [`sentinel_model`]'s untrusted-storage / validated-domain
//! split. [`capture`] and [`corpus`] are the untrusted side: [`capture`] decodes
//! a recorded `capture-envelope-v1` artifact and [`corpus`] a `trace-corpus-v1`
//! bundle, each by walking JSON explicitly and rejecting unknown and ill-typed
//! members. [`sagashop`] and [`corpus_adapter`] are the Adapters, which turn
//! those DTOs into canonical Events with explicit causal edges and honest
//! source capabilities. [`evidence`] is what the Verification Core consumes.
//!
//! The two Adapters are the two halves of the ADR-0029 class boundary: the
//! SagaShop path emits Tenant-class Events and the corpus path emits
//! Corpus-class Events, and [`sentinel_model::Event::observe`] accepts no
//! default between them.
//!
//! The recorded-file path is the whole of this slice. There is no OTLP
//! receiver, no gRPC or HTTP server, and no network: ADR-0019 licenses "OTLP
//! **or recorded evidence**", and recorded evidence is the strictly smaller
//! path and the only one that keeps CI hermetic.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.

pub mod capture;
pub mod corpus;
pub mod corpus_adapter;
pub mod evidence;
pub mod sagashop;
