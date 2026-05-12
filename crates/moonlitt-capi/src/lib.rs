#![allow(clippy::not_unsafe_ptr_arg_deref)]

//! # moonlitt-capi
//!
//! C ABI bindings for the moonlitt audio engine.
//! Produces .dll / .dylib / .so for consumption by C#, Node.js, Python, etc.
//!
//! All functions are `extern "C"`, NULL-safe, and return 0 for success.

mod engine_api;
mod runtime_api;
mod builtin_api;
mod session_api;
mod util;

pub use engine_api::*;
pub use runtime_api::*;
pub use builtin_api::*;
pub use session_api::*;
pub use util::*;
