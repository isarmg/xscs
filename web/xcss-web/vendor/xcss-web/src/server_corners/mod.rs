//! Explicit server decoration. Public product pages remain opt-in.
#[cfg(target_arch = "wasm32")]
mod controller;
pub mod geometry;
#[cfg(any(target_arch = "wasm32", test))]
mod paint;
#[cfg(target_arch = "wasm32")]
pub use controller::{ServerCornersController, start};
