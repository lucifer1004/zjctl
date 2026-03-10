//! Protocol types for zjctl RPC communication with zrpc plugin.
//!
//! Uses newline-delimited JSON (jsonl) for transport over Zellij pipes.
//! Per [[ADR-0003]], this crate is the shared protocol library between CLI and plugin.

mod protocol;
mod selector;

pub use protocol::*;
pub use selector::*;
