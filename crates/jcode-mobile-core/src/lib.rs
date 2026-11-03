//! GPUI-free core of jcode mobile. A port of the Swift `JCodeKit` so the Rust
//! and Swift apps share one behavior spec. Nothing here touches UI; the Cloud SSH link
//! runs a private tokio runtime on its own thread, so it builds unchanged for Linux, macOS and iOS.

pub mod cloud;
pub mod connection;
pub mod credentials;
pub mod gateway;
pub mod session;
pub mod wire;

pub use cloud::{Account, AccountStore};
pub use connection::{Config, Connection, Endpoint};
pub use credentials::{CredentialStore, FileCredentialStore, ServerCredential};
pub use gateway::{Gateway, PairPayload, PairResponse, parse_pair_uri};
pub use session::{
    ConnectionOutput, ConnectionPhase, LocalIntent, Role, SessionState, ToolCall, ToolStatus,
    TranscriptEntry,
};
pub use wire::{Request, ServerEvent};
