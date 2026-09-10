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
//! * `resolver` — A host-side callback invoked on every credentialed
//!   outgoing request to produce a fresh secret value. The host calls
//!   the resolver synchronously from the WASI HTTP dispatch path, so
//!   implementations should be fast and (where appropriate) memoise
//!   internally. Errors returned by the resolver surface to the guest
//!   as a request-level dispatch failure with a host-redacted message.
//!
//! The registry is populated by the host before the guest runs.
//! Guests bind a credential to a specific outgoing request via WIT
//! `attach`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Host-side callback that produces the secret token value for a
/// credential at request-dispatch time.
///
/// The returned `String` is treated as the literal token; the host
/// prepends [`CredentialEntry::prefix`] to it to form the outgoing
/// header value.
///
/// On error, the returned diagnostic string is **dropped** by the
/// outgoing-handler before any guest-visible error is produced — it
/// is neither sent to the guest nor logged by this crate. The wire
/// path surfaces only a fixed `"credential resolver failed"`
/// indication. Resolver authors who need diagnostics should record
/// them inside the resolver itself (e.g. via the host's own logger)
/// before returning the `Err`.
pub type ResolverFn = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Metadata for a single scoped credential.
#[derive(Clone)]
pub struct CredentialEntry {
    /// URL-prefix scope. Only requests whose URL starts with this
    /// value are eligible for credential injection.
    pub target: String,

    /// HTTP header name to set (e.g. `"Authorization"`).
    pub header: String,

    /// Value prefix prepended to the resolved token
    /// (e.g. `"Bearer "`). May be empty.
    pub prefix: String,

    /// Resolver callback. Invoked on every credentialed outgoing
    /// request; see [`ResolverFn`] for the contract.
    pub resolver: ResolverFn,
}

impl fmt::Debug for CredentialEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The resolver is a function pointer that may close over secret
        // material; we never want it (or its captures) to appear in a
        // log line, panic message, or `dbg!` output.
        f.debug_struct("CredentialEntry")
            .field("target", &self.target)
            .field("header", &self.header)
            .field("prefix", &self.prefix)
            .field("resolver", &"<callback>")
            .finish()
    }
}

impl CredentialEntry {
    /// Build a [`CredentialEntry`] whose resolver returns a fixed
    /// token string on every invocation.
    ///
    /// Convenience constructor for tests, examples, and trivially
    /// short-lived secrets. Production callers that need refresh
    /// behaviour (managed identities, OAuth, …) should construct
    /// the entry directly with a custom [`ResolverFn`].
    pub fn with_static_resolver(
        target: impl Into<String>,
        header: impl Into<String>,
        prefix: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        let token = token.into();
        Self {
            target: target.into(),
            header: header.into(),
            prefix: prefix.into(),
            resolver: Arc::new(move || Ok(token.clone())),
        }
    }
}

/// Shared, thread-safe credential registry keyed by credential id.
pub type CredentialRegistry = Arc<Mutex<HashMap<String, CredentialEntry>>>;

/// Creates an empty credential registry.
pub fn empty_registry() -> CredentialRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}
