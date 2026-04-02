//! # moonlitt-node
//!
//! Node.js bindings for the moonlitt audio engine via napi-rs.
//!
//! Exposes engine creation, session management, effect factories,
//! and plugin scanning to Node.js / Electron / Ink applications.

mod effects;
mod engine;
mod session;
mod types;
