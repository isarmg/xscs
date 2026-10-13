//! xscs is an independent service for managing Sunshine hosts.
//!
//! Product state and Sunshine credentials remain local; shared platform
//! metadata and administrator policy are supplied by xcss.

pub mod config;
pub mod crypto;
pub mod database_schema;
pub mod db;
pub mod error;
pub mod http;
pub mod model;
pub mod operations;
pub mod release_contract;
pub mod runtime_lock;

pub use config::ServeConfig;
pub use error::{AppError, AppResult};

pub mod web_assets;

pub mod offline;
