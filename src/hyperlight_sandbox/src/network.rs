//! Network permissions and HTTP request types.

use std::fmt;
use std::str::FromStr;

use anyhow::Result;
use url::Url;

// ---------------------------------------------------------------------------
// HTTP method
// ---------------------------------------------------------------------------

/// Standard HTTP methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Patch,
    Connect,
    Trace,
}

impl FromStr for HttpMethod {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            "PATCH" => Ok(Self::Patch),
            "CONNECT" => Ok(Self::Connect),
            "TRACE" => Ok(Self::Trace),
            _ => Err(anyhow::anyhow!("invalid HTTP method: {s}")),
        }
    }
}

impl TryFrom<&str> for HttpMethod {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        s.parse()
    }
}

impl TryFrom<String> for HttpMethod {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
            Self::Patch => "PATCH",
            Self::Connect => "CONNECT",
            Self::Trace => "TRACE",
        }
    }

    /// Maximum number of methods in a filter list.
    const MAX_METHOD_LIST: usize = 16;

    /// Parse an optional list of method strings into typed `HttpMethod` values.
    pub fn parse_list(methods: Option<Vec<String>>) -> Result<MethodFilter> {
        match methods {
            None => Ok(MethodFilter::Any),
            Some(m) => {
                if m.len() > Self::MAX_METHOD_LIST {
                    return Err(anyhow::anyhow!(
                        "too many HTTP methods ({}, max {})",
                        m.len(),
                        Self::MAX_METHOD_LIST
                    ));
                }
                let parsed: Result<Vec<HttpMethod>> =
                    m.into_iter().map(HttpMethod::try_from).collect();
                Ok(MethodFilter::Only(parsed?))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Network permissions
// ---------------------------------------------------------------------------

/// Specifies which HTTP methods a permission allows.
#[derive(Debug, Clone)]
pub enum MethodFilter {
    /// All HTTP methods are allowed.
    Any,
    /// Only the specified methods are allowed.
    Only(Vec<HttpMethod>),
}

impl From<Vec<HttpMethod>> for MethodFilter {
    fn from(methods: Vec<HttpMethod>) -> Self {
        MethodFilter::Only(methods)
    }
}

/// A single network permission entry — describes what the sandbox is allowed to reach.
///
/// Only the domain (host) and optional port are stored. URL path and scheme
/// are intentionally not checked — this is a domain-level allowlist.
#[derive(Debug, Clone)]
pub struct NetworkPermission {
    /// The parsed URL target.
    pub url: Url,
    /// Which HTTP methods are allowed.
    pub methods: MethodFilter,
}

/// Manages the set of allowed network destinations for a sandbox.
#[derive(Debug, Clone, Default)]
pub struct NetworkPermissions {
    permissions: Vec<NetworkPermission>,
}

impl NetworkPermissions {
    pub(crate) fn new() -> Self {
        Self {
            permissions: Vec::new(),
        }
    }

    /// Maximum number of network permission entries to prevent memory exhaustion.
    const MAX_PERMISSIONS: usize = 1024;

    /// Add a domain-level network permission.
    ///
    /// `target` must be a valid URL (e.g. `"https://httpbin.org"` or
    /// `"https://example.com:8080"`). Only the host and non-default port
    /// are extracted — the URL path and scheme are **not** checked at
    /// request time. This is a domain-level allowlist.
    ///
    /// `methods` specifies which HTTP methods are allowed.
    pub fn allow_domain(&mut self, target: &str, methods: impl Into<MethodFilter>) -> Result<()> {
        if self.permissions.len() >= Self::MAX_PERMISSIONS {
            return Err(anyhow::anyhow!(
                "maximum number of network permissions ({}) reached",
                Self::MAX_PERMISSIONS
            ));
        }
        let url = Url::parse(target)
            .map_err(|e| anyhow::anyhow!("invalid URL for network permission: {e}"))?;

        // Validate that the URL has a host before storing.
        url.host_str()
            .ok_or_else(|| anyhow::anyhow!("URL has no host: {target}"))?;

        self.permissions.push(NetworkPermission {
            url,
            methods: methods.into(),
        });
        Ok(())
    }

    /// HTTP methods that are always blocked regardless of permissions.
    ///
    /// CONNECT enables HTTP tunneling and TRACE can reflect credentials.
    const BLOCKED_METHODS: [HttpMethod; 2] = [HttpMethod::Connect, HttpMethod::Trace];

    /// Check if a request to the given URL + method is allowed.
    ///
    /// Checks host (exact match), scheme, path prefix, and optional port.
    /// Subdomains do **not** match a parent-domain permission.
    /// CONNECT and TRACE are always blocked.
    pub fn is_allowed(&self, url: &Url, method: &HttpMethod) -> bool {
        if Self::BLOCKED_METHODS.contains(method) {
            return false;
        }
        self.permissions.iter().any(|p| {
            // Host must match exactly (no subdomain wildcard).
            let request_host = url.host_str().unwrap_or("");
            let permission_host = p.url.host_str().unwrap_or("");
            if request_host != permission_host {
                return false;
            }

            // Scheme must match.
            if p.url.scheme() != url.scheme() {
                return false;
            }

            // Port check: if the permission specifies a port, the request
            // port must match exactly.  If the permission has no port (i.e.
            // the default port for the scheme), only requests on the default
            // port are allowed — a non-default port is rejected.
            if p.url.port() != url.port() {
                return false;
            }

            // Path prefix check.
            let permission_path = p.url.path().trim_end_matches('/');
            let request_path = url.path();
            if !permission_path.is_empty() {
                if !request_path.starts_with(permission_path) {
                    return false;
                }
                // Ensure it's a proper segment boundary, not a partial match
                // e.g. /api matches /api and /api/users but NOT /apiary
                if request_path.len() > permission_path.len()
                    && !request_path[permission_path.len()..].starts_with('/')
                {
                    return false;
                }
            }

            // Method check.
            match &p.methods {
                MethodFilter::Any => {}
                MethodFilter::Only(methods) => {
                    if !methods.contains(method) {
                        return false;
                    }
                }
            }
            true
        })
    }
}

/// Split an authority string (`"host"` or `"host:port"`) into its components.
#[cfg(test)]
fn split_authority(authority: &str) -> (&str, Option<u16>) {
    if let Some((host, port_str)) = authority.rsplit_once(':')
        && let Ok(port) = port_str.parse::<u16>()
    {
        return (host, Some(port));
    }
    (authority, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse a URL in tests.
    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    // -----------------------------------------------------------------------
    // Basic domain + method
    // -----------------------------------------------------------------------

    #[test]
    fn allowed_domain_and_method() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain(
                "https://example.com",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/"), &HttpMethod::Get));
    }

    #[test]
    fn disallowed_method() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain(
                "https://example.com",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();

        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Post));
        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Delete));
    }

    #[test]
    fn disallowed_domain() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain(
                "https://example.com",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();

        assert!(!network.is_allowed(&url("https://other.com"), &HttpMethod::Get));
    }

    #[test]
    fn any_methods_allows_all() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://httpbin.org", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://httpbin.org"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://httpbin.org"), &HttpMethod::Post));
        assert!(network.is_allowed(&url("https://httpbin.org"), &HttpMethod::Delete));
        assert!(network.is_allowed(&url("https://httpbin.org"), &HttpMethod::Put));
        assert!(network.is_allowed(&url("https://httpbin.org"), &HttpMethod::Patch));
        assert!(!network.is_allowed(&url("https://other.com"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Subdomain isolation
    // -----------------------------------------------------------------------

    #[test]
    fn subdomain_not_allowed_by_parent() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com", MethodFilter::Any)
            .unwrap();

        assert!(!network.is_allowed(&url("https://sub.example.com"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://api.example.com"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://deep.sub.example.com"), &HttpMethod::Get));
    }

    #[test]
    fn parent_not_allowed_by_subdomain() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://api.example.com", MethodFilter::Any)
            .unwrap();

        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
    }

    #[test]
    fn different_subdomain_not_allowed() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://api.example.com", MethodFilter::Any)
            .unwrap();

        assert!(!network.is_allowed(&url("https://web.example.com"), &HttpMethod::Get));
    }

    #[test]
    fn exact_subdomain_allowed() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://api.example.com", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://api.example.com"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://api.example.com/v1/data"), &HttpMethod::Post));
    }

    // -----------------------------------------------------------------------
    // Path matching
    // -----------------------------------------------------------------------

    #[test]
    fn path_prefix_allowed() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/api"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/users"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/users/123"), &HttpMethod::Get));
    }

    #[test]
    fn path_prefix_no_partial_segment_match() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api", MethodFilter::Any)
            .unwrap();

        // /apiary starts with /api but is a different segment
        assert!(!network.is_allowed(&url("https://example.com/apiary"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com/api2"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com/api-v2"), &HttpMethod::Get));
    }

    #[test]
    fn path_different_not_allowed() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api", MethodFilter::Any)
            .unwrap();

        assert!(!network.is_allowed(&url("https://example.com/web"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com/other"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com/"), &HttpMethod::Get));
    }

    #[test]
    fn root_path_allows_all_paths() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/anything"), &HttpMethod::Get));
        assert!(network.is_allowed(
            &url("https://example.com/deep/nested/path"),
            &HttpMethod::Get
        ));
        assert!(network.is_allowed(&url("https://example.com/"), &HttpMethod::Get));
    }

    #[test]
    fn deep_path_prefix() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api/v2/resources", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(
            &url("https://example.com/api/v2/resources"),
            &HttpMethod::Get
        ));
        assert!(network.is_allowed(
            &url("https://example.com/api/v2/resources/123"),
            &HttpMethod::Get
        ));
        assert!(!network.is_allowed(&url("https://example.com/api/v2"), &HttpMethod::Get));
        assert!(!network.is_allowed(
            &url("https://example.com/api/v1/resources"),
            &HttpMethod::Get
        ));
    }

    #[test]
    fn trailing_slash_on_permission_path() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api/", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/api"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/users"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Scheme checking
    // -----------------------------------------------------------------------

    #[test]
    fn scheme_must_match() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("http://example.com"), &HttpMethod::Get));
    }

    #[test]
    fn http_scheme_allowed_when_specified() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("http://example.com", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("http://example.com"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Port checking
    // -----------------------------------------------------------------------

    #[test]
    fn port_must_match_when_specified() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com:8443", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com:8443"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com:9443"), &HttpMethod::Get));
    }

    #[test]
    fn no_port_in_permission_rejects_non_default_port() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com", MethodFilter::Any)
            .unwrap();

        // Default port (443 for https) is allowed
        assert!(network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        // Explicit default port is equivalent, so also allowed
        assert!(network.is_allowed(&url("https://example.com:443"), &HttpMethod::Get));
        // Non-default ports must be rejected
        assert!(!network.is_allowed(&url("https://example.com:8443"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Multiple permissions
    // -----------------------------------------------------------------------

    #[test]
    fn multiple_permissions_any_match() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain(
                "https://api.example.com/v1",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();
        network
            .allow_domain(
                "https://api.example.com/v2",
                MethodFilter::Only(vec![HttpMethod::Post]),
            )
            .unwrap();

        assert!(network.is_allowed(&url("https://api.example.com/v1/users"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://api.example.com/v1/users"), &HttpMethod::Post));
        assert!(network.is_allowed(&url("https://api.example.com/v2/data"), &HttpMethod::Post));
        assert!(!network.is_allowed(&url("https://api.example.com/v2/data"), &HttpMethod::Get));
    }

    #[test]
    fn multiple_domains() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com", MethodFilter::Any)
            .unwrap();
        network
            .allow_domain("https://httpbin.org", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://httpbin.org"), &HttpMethod::Post));
        assert!(!network.is_allowed(&url("https://evil.com"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Path + method combined
    // -----------------------------------------------------------------------

    #[test]
    fn path_and_method_combined() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain(
                "https://example.com/api",
                MethodFilter::Only(vec![HttpMethod::Get, HttpMethod::Post]),
            )
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/api/data"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/data"), &HttpMethod::Post));
        assert!(!network.is_allowed(&url("https://example.com/api/data"), &HttpMethod::Delete));
        assert!(!network.is_allowed(&url("https://example.com/web"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Query strings
    // -----------------------------------------------------------------------

    #[test]
    fn query_strings_ignored_in_path_match() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/api?key=value"), &HttpMethod::Get));
        assert!(network.is_allowed(
            &url("https://example.com/api/users?page=1&limit=10"),
            &HttpMethod::Get
        ));
    }

    // -----------------------------------------------------------------------
    // Empty / no permissions
    // -----------------------------------------------------------------------

    #[test]
    fn no_permissions_denies_all() {
        let network = NetworkPermissions::new();
        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn allow_domain_rejects_invalid_url() {
        let mut network = NetworkPermissions::new();
        assert!(
            network
                .allow_domain("not a url", MethodFilter::Any)
                .is_err()
        );
    }

    #[test]
    fn url_with_fragment() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/api#section"), &HttpMethod::Get));
    }

    #[test]
    fn url_with_userinfo_still_matches_host() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://user:pass@example.com/path"), &HttpMethod::Get));
    }

    #[test]
    fn all_http_methods_checked() {
        let mut network = NetworkPermissions::new();
        let allowed_methods = vec![
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Delete,
            HttpMethod::Head,
            HttpMethod::Options,
            HttpMethod::Patch,
        ];
        // CONNECT and TRACE are always blocked for security reasons,
        // even if explicitly listed in the permission filter.
        let blocked_methods = vec![HttpMethod::Connect, HttpMethod::Trace];

        let all_methods: Vec<_> = allowed_methods
            .iter()
            .chain(&blocked_methods)
            .cloned()
            .collect();
        network
            .allow_domain("https://example.com", MethodFilter::Only(all_methods))
            .unwrap();

        for m in &allowed_methods {
            assert!(
                network.is_allowed(&url("https://example.com"), m),
                "method {m} should be allowed"
            );
        }
        for m in &blocked_methods {
            assert!(
                !network.is_allowed(&url("https://example.com"), m),
                "method {m} should be blocked"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Permission URL edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn permission_with_query_string_is_ignored_in_path() {
        let mut network = NetworkPermissions::new();
        // Query strings in permission URLs are parsed by url::Url but
        // is_allowed only checks path(), which excludes query.
        network
            .allow_domain("https://example.com/api?key=secret", MethodFilter::Any)
            .unwrap();

        // Path is /api — should match requests to /api regardless of query
        assert!(network.is_allowed(&url("https://example.com/api"), &HttpMethod::Get));
        assert!(network.is_allowed(
            &url("https://example.com/api?other=value"),
            &HttpMethod::Get
        ));
        assert!(network.is_allowed(&url("https://example.com/api/sub"), &HttpMethod::Get));
    }

    #[test]
    fn permission_with_fragment_is_ignored() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/api#section", MethodFilter::Any)
            .unwrap();

        // Fragments are stripped by url::Url, path is /api
        assert!(network.is_allowed(&url("https://example.com/api"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/users"), &HttpMethod::Get));
    }

    #[test]
    fn permission_with_userinfo() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://user:pass@example.com", MethodFilter::Any)
            .unwrap();

        // host_str() returns "example.com" regardless of userinfo
        assert!(network.is_allowed(&url("https://example.com/path"), &HttpMethod::Get));
        assert!(network.is_allowed(
            &url("https://other:creds@example.com/path"),
            &HttpMethod::Get
        ));
    }

    #[test]
    fn permission_with_explicit_default_port() {
        let mut network = NetworkPermissions::new();
        // url::Url normalizes :443 on https to None
        network
            .allow_domain("https://example.com:443", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com:443"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com:8443"), &HttpMethod::Get));
    }

    #[test]
    fn permission_with_explicit_default_http_port() {
        let mut network = NetworkPermissions::new();
        // url::Url normalizes :80 on http to None
        network
            .allow_domain("http://example.com:80", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("http://example.com"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("http://example.com:80"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("http://example.com:8080"), &HttpMethod::Get));
    }

    #[test]
    fn permission_with_non_default_port_rejects_default_port() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com:8443", MethodFilter::Any)
            .unwrap();

        // Only :8443 should be allowed, not default port
        assert!(network.is_allowed(&url("https://example.com:8443"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com:443"), &HttpMethod::Get));
    }

    #[test]
    fn permission_url_with_encoded_path() {
        let mut network = NetworkPermissions::new();
        network
            .allow_domain("https://example.com/my%20api", MethodFilter::Any)
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com/my%20api"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/my%20api/data"), &HttpMethod::Get));
        assert!(!network.is_allowed(&url("https://example.com/my"), &HttpMethod::Get));
    }

    #[test]
    fn network_permissions_port_matching() {
        let mut network = NetworkPermissions::new();
        // Non-default port — must match exactly.
        network
            .allow_domain(
                "https://example.com:8080",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com:8080"), &HttpMethod::Get));
        // Wrong port → denied.
        assert!(!network.is_allowed(&url("https://example.com:9090"), &HttpMethod::Get));
        // No port in request → denied when permission specifies one.
        assert!(!network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
    }

    #[test]
    fn network_permissions_default_port_allows_any() {
        let mut network = NetworkPermissions::new();
        // Default port (443 for https) → Url::port() returns None, so
        // requests without an explicit port also match.
        network
            .allow_domain(
                "https://example.com",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();

        assert!(network.is_allowed(&url("https://example.com"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com:443"), &HttpMethod::Get));
        // Non-default port → denied when permission uses default port.
        assert!(!network.is_allowed(&url("https://example.com:8080"), &HttpMethod::Get));
    }

    #[test]
    fn network_permissions_path_ignored() {
        let mut network = NetworkPermissions::new();
        // Path in the permission URL — requests with any path under it should match.
        network
            .allow_domain(
                "https://example.com/api/v1",
                MethodFilter::Only(vec![HttpMethod::Get]),
            )
            .unwrap();

        // Path prefix matches.
        assert!(network.is_allowed(&url("https://example.com/api/v1"), &HttpMethod::Get));
        assert!(network.is_allowed(&url("https://example.com/api/v1/users"), &HttpMethod::Get));
    }

    #[test]
    fn split_authority_parses_host_and_port() {
        assert_eq!(split_authority("example.com"), ("example.com", None));
        assert_eq!(
            split_authority("example.com:8080"),
            ("example.com", Some(8080))
        );
        assert_eq!(
            split_authority("example.com:443"),
            ("example.com", Some(443))
        );
        // Invalid port falls back to treating whole string as host.
        assert_eq!(
            split_authority("example.com:abc"),
            ("example.com:abc", None)
        );
    }
}
