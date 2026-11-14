//! GPUI-free core of jcode mobile. A port of the Swift `JCodeKit` so the Rust
//! and Swift apps share one behavior spec. Nothing here touches UI; the Cloud SSH link
//! runs a private tokio runtime on its own thread, so it builds unchanged for Linux, macOS and iOS.

pub mod agents;
pub mod apple;
pub mod cloud;
pub mod cloud_login;
pub mod cloud_machine;
pub mod cloud_ssh;
mod cloud_direct;
pub mod mobile_sync;
pub mod prefs;
pub mod connection;
pub mod crash;
pub mod diagnostics;
pub mod credentials;
pub mod gateway;
pub mod git_info;
pub mod routes;
pub mod voice;
pub mod session;
pub mod timing;
pub mod transcript_cache;
pub mod todos;

/// The harness API client this core is built on.
pub use jcode_sdk;

pub use cloud::{Account, AccountStore};
pub use connection::{Config, Connection, Endpoint, Request, TransportFactory};
pub use credentials::{CredentialStore, FileCredentialStore, ServerCredential};
pub use prefs::{Prefs, PrefsStore};
pub use gateway::{Gateway, PairPayload, PairResponse, parse_pair_uri};
pub use session::{
    ConnectionOutput, ConnectionPhase, LocalIntent, Role, SessionState, ToolCall, ToolStatus,
    TranscriptEntry,
};
pub use jcode_sdk::ApiEvent;
