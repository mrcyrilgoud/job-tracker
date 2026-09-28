//! Structured, runtime-configurable job filtering.
//!
//! This module houses the pure filter engine and its supporting pieces:
//! - [`model`]: the versioned, serializable `FilterCriteria` data model.
//! - [`aliases`]: the data-driven alias table (regions, countries, remote tokens).
//! - [`engine`]: the single pure matching authority.
//! - [`resolver`]: resolution of effective criteria from global/per-watch scopes.
//!
//! Submodules are declared here; their implementations are filled in by later tasks.

pub mod aliases;
pub mod engine;
pub mod model;
pub mod resolver;
