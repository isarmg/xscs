//! xcss server support in one Linux AMD64 crate.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]

#[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
compile_error!("xcss supports only x86_64-unknown-linux-gnu server builds");

extern crate self as xcss;

pub mod admin_auth;
pub mod admin_axum;
pub mod admin_core;
pub mod admin_hyper;
pub mod admin_sqlite;
pub mod config;
pub mod configuration_contract;
pub mod contracts;
pub mod error;
pub mod fs_safety;
pub mod installation_material;
pub mod log;
pub mod operations;
pub mod platform_db;
pub mod schema_identity;
pub mod secret;
pub mod secret_envelope;
pub mod secure_http;
pub mod server_cli;
pub mod server_layout;
pub mod server_runtime;
pub mod server_secrets;
pub mod server_target;
pub mod sqlite;
pub mod state_file;
pub mod testkit;
pub mod web_assets;
