//! A decompiler focused on CLDC/MIDP-era bytecode.
//!
//! Class identities remain in their original JVM namespace throughout analysis.
//! Source names are allocated once, before independently processing methods.

#![forbid(unsafe_code)]

pub use anyhow::Result;

pub mod api;
pub mod bytecode;
pub mod cfg;

pub mod classfile;
pub mod hash;
pub mod index;
pub mod initialization;
pub mod input;
pub mod ir;
pub mod java;
pub mod mappings;
pub mod naming;
mod normal_returns;
pub mod program;
#[cfg(not(target_arch = "wasm32"))]
pub mod remap;
pub mod semantics;
pub mod session;
pub mod structure;
pub mod throws;
