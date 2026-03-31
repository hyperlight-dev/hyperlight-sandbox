use std::str::FromStr as _;

use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use hyperlight_sandbox::http as sandbox_http;

#[derive(Default, Clone)]
pub struct Headers {
    inner: HeaderMap,
    pub immutable: bool,
}

pub enum HeaderError {
    Immutable,
    InvalidHeader,
    Forbidden,
    TooMany,
}

impl From<InvalidHeaderName> for HeaderError {
    fn from(_: InvalidHeaderName) -> Self {
        HeaderError::InvalidHeader
    }
}

impl From<InvalidHeaderValue> for HeaderError {
    fn from(_: InvalidHeaderValue) -> Self {
        HeaderError::InvalidHeader
    }
}

impl From<HeaderMap> for Headers {
    fn from(inner: HeaderMap) -> Self {
        Self {
            inner,
            immutable: false,
        }
    }
}

impl Headers {
    fn is_forbidden(name: &HeaderName) -> bool {
        sandbox_http::is_forbidden_request_header(name.as_str())
    }

    fn total_bytes(&self) -> usize {
        self.inner
            .iter()
            .map(|(k, v)| k.as_str().len() + v.as_bytes().len())
            .sum()
    }

    /// Create an immutable `Headers` from an [`http::HeaderMap`].
    ///
    /// The caller is expected to have already applied header limits (e.g.
    /// via [`sandbox_http::send_http_request`] which caps count, byte-size,
    /// and strips forbidden *request* headers).  This method simply wraps
    /// the map as immutable for the WASI guest.
    pub fn from_http_headers(headers: HeaderMap) -> Self {
        Self {
            inner: headers,
            immutable: true,
        }
    }

    pub fn from_list(
        entries: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Self, HeaderError> {
        let mut headers = HeaderMap::new();
        let mut total_bytes: usize = 0;
        for (k, v) in entries {
            if headers.len() >= sandbox_http::MAX_RESPONSE_HEADER_COUNT {
                return Err(HeaderError::TooMany);
            }
            total_bytes = total_bytes.saturating_add(k.len() + v.len());
            if total_bytes > sandbox_http::MAX_RESPONSE_HEADER_BYTES {
                return Err(HeaderError::TooMany);
            }
            let name = HeaderName::from_str(&k)?;
            if Self::is_forbidden(&name) {
                return Err(HeaderError::Forbidden);
            }
            let value = HeaderValue::from_bytes(&v)?;
            headers.append(name, value);
        }
        Ok(Self {
            inner: headers,
            immutable: false,
        })
    }

    pub fn get(&self, name: impl AsRef<str>) -> Result<Vec<Vec<u8>>, HeaderError> {
        let name = HeaderName::from_str(name.as_ref())?;
        let values = self
            .inner
            .get_all(name)
            .iter()
            .map(|x| x.as_bytes().to_vec())
            .collect();
        Ok(values)
    }

    pub fn has(&self, name: impl AsRef<str>) -> bool {
        let Ok(name) = HeaderName::from_str(name.as_ref()) else {
            return false;
        };
        self.inner.contains_key(name)
    }

    pub fn set(
        &mut self,
        name: impl AsRef<str>,
        values: impl IntoIterator<Item = impl AsRef<[u8]>>,
    ) -> Result<(), HeaderError> {
        if self.immutable {
            return Err(HeaderError::Immutable);
        }
        let name = HeaderName::from_str(name.as_ref())?;
        if Self::is_forbidden(&name) {
            return Err(HeaderError::Forbidden);
        }
        let values = values
            .into_iter()
            .map(|val| HeaderValue::from_bytes(val.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        self.inner.remove(&name);
        for val in values {
            self.inner.append(&name, val);
        }
        Ok(())
    }

    pub fn delete(&mut self, name: impl AsRef<str>) -> Result<(), HeaderError> {
        if self.immutable {
            return Err(HeaderError::Immutable);
        }
        let name = HeaderName::from_str(name.as_ref())?;
        if Self::is_forbidden(&name) {
            return Err(HeaderError::Forbidden);
        }
        self.inner.remove(name);
        Ok(())
    }

    pub fn append(
        &mut self,
        name: impl AsRef<str>,
        value: impl AsRef<[u8]>,
    ) -> Result<(), HeaderError> {
        if self.immutable {
            return Err(HeaderError::Immutable);
        }
        if self.inner.len() >= sandbox_http::MAX_RESPONSE_HEADER_COUNT {
            return Err(HeaderError::TooMany);
        }
        let name = HeaderName::from_str(name.as_ref())?;
        if Self::is_forbidden(&name) {
            return Err(HeaderError::Forbidden);
        }
        let value_ref = value.as_ref();
        if self.total_bytes() + name.as_str().len() + value_ref.len()
            > sandbox_http::MAX_RESPONSE_HEADER_BYTES
        {
            return Err(HeaderError::TooMany);
        }
        let value = HeaderValue::from_bytes(value_ref)?;
        self.inner.append(name, value);
        Ok(())
    }

    pub fn entries(&self) -> Vec<(String, Vec<u8>)> {
        self.inner
            .iter()
            .map(|(k, v)| (k.as_str().into(), v.as_bytes().to_vec()))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
