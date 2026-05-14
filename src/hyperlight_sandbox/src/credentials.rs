//! Scoped-credential registry for outgoing HTTP requests.
//!
//! A [`CredentialEntry`] binds a logical credential identifier to the
//! metadata required to inject a token header at request time:
//!
//! * `target` — URL-prefix scope. The outgoing-handler only injects
//!   the credential when the request URL starts with this prefix.
//! * `header` — HTTP header name (e.g. `"Authorization"`).
//! * `prefix` — Value prefix prepended to the resolved token
//!   (e.g. `"Bearer "`).
//! * `resolver` — Opaque resolver identifier. Today this is a simple
//!   string key; a future commit will support async resolution
//!   callbacks.
//!
//! The registry is populated by the host before the guest runs.
//! Guests bind a credential to a specific outgoing request via WIT
//! `attach`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Metadata for a single scoped credential.
#[derive(Debug, Clone)]
pub struct CredentialEntry {
    /// URL-prefix scope. Only requests whose URL starts with this
    /// value are eligible for credential injection.
    pub target: String,

    /// HTTP header name to set (e.g. `"Authorization"`).
    pub header: String,

    /// Value prefix prepended to the resolved token
    /// (e.g. `"Bearer "`). May be empty.
    pub prefix: String,

    /// Opaque resolver identifier. The outgoing-handler will use
    /// this to obtain the actual secret value at dispatch time.
    pub resolver: String,
}

/// Shared, thread-safe credential registry keyed by credential id.
pub type CredentialRegistry = Arc<Mutex<HashMap<String, CredentialEntry>>>;

/// Creates an empty credential registry.
pub fn empty_registry() -> CredentialRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}
