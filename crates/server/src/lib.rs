//! xscs is an independent service for managing Sunshine hosts.
//!
//! Product state and Sunshine credentials remain local; shared platform
//! metadata and administrator policy are supplied by xcss.

pub mod config;
mod config_definition;
pub mod crypto;
mod database_definition;
pub mod database_schema;
pub mod db;
pub mod error;
pub mod http;
pub mod model;
pub mod operations;
mod persistent_crypto_definition;
pub mod release_contract;
pub mod runtime_lock;
pub mod runtime_validation;
mod secret_definition;

pub use config::ServeConfig;
pub use error::{AppError, AppResult};

pub mod web_assets;

#[cfg(test)]
#[path = "../tests/support/administrator.rs"]
mod test_support;
