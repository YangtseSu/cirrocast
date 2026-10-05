// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The single outbound HTTP path.
//!
//! Every request in `cirrocast` — geocoding, IP location, weather providers — is built as an
//! [`HttpRequest`] and handed to one [`HttpClient`], which owns the retry policy and the error
//! taxonomy. Two [`Transport`] implementations exist: [`UreqTransport`] for real traffic and
//! [`StubTransport`] for tests, so no test in this repository opens a socket.
//!
//! The pieces that make retries testable rather than timing-dependent:
//!
//! * the retry count is fixed by the caller (`min(1 + network.retries, 3)` attempts) and every
//!   wait is requested from an injected [`crate::cache::Clock`], so a test asserts the
//!   schedule instead of sleeping through it;
//! * `http_status_as_error(false)` keeps the status *and* the body of a 4xx/5xx response, which is
//!   what lets the policy retry `429`/`5xx` and report the upstream's own `reason` text;
//! * [`HttpRequest::normalized`] is the cache key input: query pairs sorted and encoded, header
//!   names lower-cased, so two spellings of the same request hash to one cache entry;
//! * [`HttpRequest::secret`] marks a credential so that no log line, error message or cache
//!   envelope can contain it — the redacted spellings are the only ones printed anywhere;
//! * `CIRROCAST_FORBID_NETWORK` is enforced by [`UreqTransport`] before DNS or connect, so the
//!   CLI's test suite can prove that no test reaches the network; loopback stays reachable so a
//!   test can aim a provider's base URL at an in-process stub;
//! * [`UreqTransport`] is built without `ureq`'s transparent content decoders and with redirects
//!   allowed only for a request that carries no header, so [`MAX_BODY_BYTES`] bounds what is read
//!   off the socket, a canonical `3xx` (the FPAS server's `/alert/<id>` → `/cap/alerts/…`) still
//!   resolves, and a credential header can never survive a hop to another authority; a
//!   non-identity `Content-Encoding` on a `2xx` is refused as an [`Error::Upstream`] naming the
//!   cap.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::de::DeserializeOwned;
use ureq::Agent;
use ureq::ResponseExt as _;

use crate::cache::Clock;
use crate::config::Network;
use crate::error::{Error, Result};

/// The `User-Agent` every request carries: identified, versioned, with a contact URL.
///
/// Nominatim's usage policy requires exactly this, and the Open-Meteo and ipwho.is endpoints are
/// equally donated services, so no request of ours goes out anonymously.
pub const UA: &str = concat!(
    "cirrocast/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/YangtseSu/cirrocast)"
);

/// The attempt cap, including the first try. `min(1 + network.retries, MAX_ATTEMPTS)`.
const MAX_ATTEMPTS: u32 = 3;

/// The first backoff step; every further attempt doubles it.
const BACKOFF_STEP: Duration = Duration::from_millis(500);

/// Upper bound for an upstream `Retry-After`, so a hostile header cannot hang the CLI.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The redirect hops a header-free request may follow. Small on purpose: the only canonical
/// redirect in the registry is FPAS's `/alert/<id>` → `/cap/alerts/…`, so a longer chain means
/// an upstream worth refusing rather than chasing.
const MAX_REDIRECTS: u32 = 5;

/// The most bytes one response body may occupy. Every upstream this project talks to answers in
/// kilobytes; the cap turns a runaway or hostile body into a typed error instead of an allocation
/// the process cannot survive. (It also replaces `ureq`'s implicit 10 MiB default with a named,
/// tested one.)
pub const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

/// The environment variable that disables outbound traffic for a whole run.
const FORBID_NETWORK_ENV: &str = "CIRROCAST_FORBID_NETWORK";

/// Whether this process must not open outbound connections.
///
/// Read once ([`LazyLock`]): the switch exists so CI and the test suite can prove that nothing
/// reaches the network, and a per-request toggle would let a late read flip it back on.
/// A non-empty value other than `0` enables the guard; `0` and the empty string leave it off.
static FORBIDDEN: LazyLock<bool> = LazyLock::new(|| {
    std::env::var(FORBID_NETWORK_ENV).is_ok_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0"
    })
});

/// Whether [`FORBIDDEN`] is on, as a function so call sites read as intent rather than as a deref.
fn network_forbidden() -> bool {
    *FORBIDDEN
}

/// Whether `url` addresses the local machine, and therefore stays reachable with the network guard
/// on. Loopback is exempt so a test can point a provider's base URL at an in-process stub; anything
/// else — including a name that would need DNS — is refused before a socket is opened.
fn is_loopback_url(url: &str) -> bool {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // Strip any `user:password@` userinfo before the authority is split: `http://localhost@evil.com`
    // names `evil.com`, so an un-stripped authority would misclassify it as loopback.
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    // Bracketed IPv6 literals (`[::1]:8080`) take precedence over the `:`-splitting.
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

/// Characters left unescaped in a query string: the RFC 3986 unreserved set.
const QUERY_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The HTTP method; only `GET` is needed by every endpoint this project talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`
    Get,
}

impl Method {
    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
        }
    }
}

/// One request, before any transport sees it.
///
/// A request may carry a credential — in a query parameter, a header or the path itself — and every
/// log line, error message and cache envelope must stay free of it. [`HttpRequest::secret`] records
/// such a value; the redacted spellings ([`HttpRequest::redacted_url`],
/// [`HttpRequest::redacted_normalized`], the [`Debug`] output) are what the rest of the program
/// prints.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    method: Method,
    url: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    secrets: Vec<String>,
}

/// The placeholder a secret is replaced with.
const REDACTED: &str = "***";

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.redacted_url())
            .field("headers", &self.headers.len())
            .field("secrets", &self.secrets.len())
            .finish_non_exhaustive()
    }
}

impl HttpRequest {
    /// A `GET` for `url`, without a query string or extra headers.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            query: Vec::new(),
            headers: Vec::new(),
            secrets: Vec::new(),
        }
    }

    /// Records `value` as a secret: it is replaced with `***` in every URL, log line, error
    /// message and cache envelope this request produces.
    ///
    /// The value itself is never used to send anything — it is already part of the URL, query or
    /// headers; this only tells the redaction where to look.
    #[must_use]
    pub fn secret(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.is_empty() {
            self.secrets.push(value);
        }
        self
    }

    /// Appends one query parameter; insertion order is the order on the wire.
    #[must_use]
    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }

    /// Appends one header.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The method.
    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }

    /// The bare URL, without the query string.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The query parameters, in wire order.
    #[must_use]
    pub fn query_pairs(&self) -> &[(String, String)] {
        &self.query
    }

    /// The extra headers, in wire order.
    #[must_use]
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The URL a transport actually fetches: query pairs percent-encoded, in insertion order.
    #[must_use]
    pub fn full_url(&self) -> String {
        if self.query.is_empty() {
            return self.url.clone();
        }
        let pairs: Vec<String> = self
            .query
            .iter()
            .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
            .collect();
        format!("{}?{}", self.url, pairs.join("&"))
    }

    /// [`Self::full_url`] with every recorded secret replaced by `***`.
    #[must_use]
    pub fn redacted_url(&self) -> String {
        self.redact(&self.full_url())
    }

    /// [`Self::normalized`] with every recorded secret replaced by `***`, so a credential can
    /// never be written into a cache envelope or a `-v` line.
    #[must_use]
    pub fn redacted_normalized(&self) -> String {
        self.redact(&self.normalized())
    }

    /// Replaces each recorded secret with [`REDACTED`], longest first so overlapping values cannot
    /// leave a tail behind.
    ///
    /// Both the raw spelling and the percent-encoded one are replaced: a credential that reaches
    /// the wire inside a query string is spelled encoded in every URL this module prints, and a
    /// secret containing `+`, `@` or any other character outside the RFC 3986 unreserved set would
    /// otherwise survive redaction.
    fn redact(&self, text: &str) -> String {
        let mut secrets: Vec<String> = self
            .secrets
            .iter()
            .filter(|value| !value.is_empty())
            .flat_map(|value| [value.clone(), encode(value)])
            .collect();
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        secrets.dedup();
        secrets.into_iter().fold(text.to_owned(), |text, secret| {
            text.replace(&secret, REDACTED)
        })
    }

    /// The canonical spelling of this request, and the input to a cache key: method, URL, query
    /// pairs sorted by key (then value) and encoded, then headers sorted with lower-cased names.
    ///
    /// Two requests that mean the same thing therefore hash to the same cache entry, whatever the
    /// order their parameters were assembled in.
    #[must_use]
    pub fn normalized(&self) -> String {
        let mut pairs: Vec<&(String, String)> = self.query.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let query: Vec<String> = pairs
            .iter()
            .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
            .collect();
        let mut headers: Vec<String> = self
            .headers
            .iter()
            .map(|(name, value)| format!("{}: {}", name.to_ascii_lowercase(), value))
            .collect();
        headers.sort();

        let mut text = format!("{} {}", self.method.as_str(), self.url);
        if !query.is_empty() {
            text.push('?');
            text.push_str(&query.join("&"));
        }
        if !headers.is_empty() {
            text.push('\n');
            text.push_str(&headers.join("\n"));
        }
        text
    }
}

/// A response the client decided to accept.
///
/// The body is kept as raw bytes because one endpoint answers with a ZIP archive (the city-dump
/// download, step 18b); every other consumer reads it as text through [`HttpResponse::body`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    url: String,
}

impl HttpResponse {
    /// The status code.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// The body, as text: the wire formats this project consumes are JSON, XML and plain text.
    ///
    /// A body that is not valid UTF-8 (only the city-dump download produces one) reads as an empty
    /// string here — use [`HttpResponse::bytes`] for it.
    #[must_use]
    pub fn body(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or_default()
    }

    /// The raw body bytes, for the one endpoint that answers with an archive.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// The URL the response came from, after any redirect the transport followed.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A header value, matched case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Every header, as received.
    #[must_use]
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Deserialises the body; a body that does not parse is an [`Error::Upstream`] naming the
    /// service, never a panic.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(self.body()).map_err(|error| Error::Upstream {
            provider: host_of(&self.url),
            status: Some(self.status),
            message: format!("cannot parse the response body as JSON: {error}"),
        })
    }

    /// The `Retry-After` delay, when the header is present and usable: delta-seconds or an
    /// HTTP-date in the future. A negative, past or unparsable value is `None`, so the caller
    /// falls back to the exponential schedule.
    #[must_use]
    pub fn retry_after(&self, now: SystemTime) -> Option<Duration> {
        let value = self.header("retry-after")?.trim();
        if let Ok(seconds) = value.parse::<i64>() {
            return u64::try_from(seconds).ok().map(Duration::from_secs);
        }
        let at: SystemTime = chrono::DateTime::parse_from_rfc2822(value).ok()?.into();
        at.duration_since(now).ok()
    }
}

/// Why a transport did not produce a response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// A configured timeout expired: connect, response headers or body.
    #[error("timeout while connecting")]
    Timeout,
    /// The connection could not be established at all.
    #[error("connection failed: {0}")]
    Connect(String),
    /// The peer closed an established connection mid-request.
    #[error("the connection was reset")]
    Reset,
    /// The host name could not be resolved.
    #[error("DNS failure: {0}")]
    Dns(String),
    /// The TLS handshake or certificate check failed.
    #[error("TLS failure: {0}")]
    Tls(String),
    /// A non-loopback connection was refused because `CIRROCAST_FORBID_NETWORK` is set: no DNS
    /// query, no socket.
    #[error("outbound network access is disabled by CIRROCAST_FORBID_NETWORK")]
    Blocked,
    /// The response body is larger than [`MAX_BODY_BYTES`].
    #[error("the response body exceeds the {MAX_BODY_BYTES} byte cap")]
    TooLarge,
    /// Any other I/O or protocol failure.
    #[error("I/O failure: {0}")]
    Io(String),
}

impl TransportError {
    /// Whether trying again can plausibly succeed.
    ///
    /// Transient conditions retry; a broken TLS setup, a blocked connection or a protocol error
    /// does not, because the next attempt would fail identically.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Connect(_) | Self::Reset | Self::Dns(_)
        )
    }
}

/// One way of executing an [`HttpRequest`].
pub trait Transport: Send + Sync {
    /// Performs the request, or reports why it could not.
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// The proxy to use: `[network] proxy` when set, otherwise the environment
/// (`HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY`). A configured URL that does not parse is a
/// configuration error, not a silent fallback.
///
/// The environment is checked for a SOCKS URL first: `ureq` accepts `socks5://` and then ignores
/// it with only a `log` line (this crate installs no logger), so the request would go out direct
/// with no warning anywhere. Recorded step-12 decision: SOCKS URLs are rejected.
fn resolve_proxy(config: &Network) -> Result<Option<ureq::Proxy>> {
    if config.proxy.trim().is_empty() {
        if let Some(message) = socks_env_proxy() {
            return Err(Error::Config(message));
        }
        return Ok(ureq::Proxy::try_from_env());
    }
    let url = config.proxy.trim();
    ureq::Proxy::new(url).map(Some).map_err(|error| {
        Error::Config(format!("network.proxy `{url}` is not a proxy URL: {error}"))
    })
}

/// The first environment proxy variable whose value is a SOCKS URL, as a [`Error::Config`] message,
/// or `None` when none of them is.
///
/// The variables and their order mirror `ureq::Proxy::try_from_env`; a value `ureq` itself would
/// reject is skipped, so a malformed `ALL_PROXY` does not mask a usable `HTTPS_PROXY`.
fn socks_env_proxy() -> Option<String> {
    const VARIABLES: [&str; 6] = [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ];
    for variable in VARIABLES {
        let Ok(value) = std::env::var(variable) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let Ok(proxy) = ureq::Proxy::new(&value) else {
            continue;
        };
        if is_socks(proxy.protocol()) {
            return Some(format!(
                "{variable} `{value}` is a SOCKS proxy, which cirrocast does not support; \
                 use an `http://` or `https://` proxy (in `[network] proxy` or the environment)"
            ));
        }
        return None;
    }
    None
}

/// Whether a proxy protocol is one of the SOCKS family this crate refuses.
fn is_socks(protocol: ureq::ProxyProtocol) -> bool {
    matches!(
        protocol,
        ureq::ProxyProtocol::Socks4
            | ureq::ProxyProtocol::Socks4A
            | ureq::ProxyProtocol::Socks5
            | ureq::ProxyProtocol::Socks5h
    )
}

/// Shared transports: an `Arc` around one is itself a transport, which is what lets a test keep a
/// handle on the [`StubTransport`] it handed to the client.
impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        (**self).execute(request)
    }
}

/// The real transport: `ureq` over rustls, with an agent per redirect policy.
pub struct UreqTransport {
    /// Requests that carry no header: a `3xx` is followed, so an upstream that answers a
    /// canonical same-or-cross-host redirect keeps working (the public FPAS server answers
    /// `301 /alert/<id>` → `/cap/alerts/…`).
    plain: Agent,
    /// Requests that carry a header: redirects are refused, because `ureq` keeps custom headers
    /// across a hop (it strips only `Authorization`, `Cookie` and `Content-Length`), so a `3xx`
    /// would deliver a provider's credential (`QWeather`'s `X-QW-Api-Key`, a `MeteoAlarm` token) to
    /// whatever authority the hop names. A `3xx` reaches the status mapping instead and becomes an
    /// [`Error::Upstream`] naming the provider.
    credentialed: Agent,
}

/// Builds one agent with the shared policy and the given redirect ceiling.
fn build_agent(config: &Network, timeout: Duration, max_redirects: u32) -> Result<Agent> {
    // `max_redirects(0)` is the credential-safe default; the plain agent follows a bounded chain.
    if max_redirects > MAX_REDIRECTS {
        return Err(Error::Config(format!(
            "internal: {max_redirects} redirects exceeds the {MAX_REDIRECTS} hop limit"
        )));
    }
    let proxy = resolve_proxy(config)?;
    Ok(Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(UA)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout))
        .timeout_recv_response(Some(timeout))
        .timeout_recv_body(Some(timeout))
        .max_redirects(max_redirects)
        .proxy(proxy)
        .build()
        .new_agent())
}

/// Whether a response to `request` may be followed to another URL.
///
/// Only a request with no header at all may follow one: any header is a credential (or carries
/// one) and `ureq` would forward it. The registry's credentialed sources are `QWeather`'s
/// `X-QW-Api-Key` and the `MeteoAlarm` bearer token; every other request is a public URL whose
/// canonical redirects (FPAS) must keep working.
#[must_use]
fn follows_redirects(request: &HttpRequest) -> bool {
    request.headers().is_empty()
}

impl UreqTransport {
    /// Builds the two agents: statuses are *not* errors (the retry policy needs the body), the
    /// user agent is [`UA`], `timeout` bounds the whole request as well as each phase (connect,
    /// response headers, body), and the proxy comes from `[network] proxy` or, when that is
    /// empty, from the `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` environment. A credentialed request
    /// never follows a redirect (see [`follows_redirects`]).
    pub fn new(config: &Network, timeout: Duration) -> Result<Self> {
        Ok(Self {
            plain: build_agent(config, timeout, MAX_REDIRECTS)?,
            credentialed: build_agent(config, timeout, 0)?,
        })
    }
}

impl Transport for UreqTransport {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        // The guard runs first, so a forbidden request is refused before DNS resolution or a
        // connect attempt — `strace` on a guarded run shows no `socket(` call at all.
        if network_forbidden() && !is_loopback_url(&request.full_url()) {
            return Err(TransportError::Blocked);
        }

        let agent = if follows_redirects(request) {
            &self.plain
        } else {
            &self.credentialed
        };
        let mut builder = match request.method() {
            Method::Get => agent.get(request.full_url()),
        };
        for (name, value) in request.headers() {
            builder = builder.header(name, value);
        }

        let mut response = builder.call().map_err(ureq_error)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let url = response.get_uri().to_string();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_vec()
            .map_err(ureq_error)?;
        Ok(HttpResponse {
            status,
            headers,
            body,
            url,
        })
    }
}

/// A canned reply, scripted by tests.
#[derive(Debug, Clone)]
pub struct StubReply {
    status: u16,
    headers: Vec<(String, String)>,
    body: std::result::Result<String, TransportError>,
}

impl StubReply {
    /// A `200`-style reply with the given status and an empty header set.
    pub fn ok(status: u16, body: impl Into<String>) -> Self {
        Self::status(status, Vec::new(), body)
    }

    /// A reply with explicit status and headers.
    pub fn status(status: u16, headers: Vec<(String, String)>, body: impl Into<String>) -> Self {
        Self {
            status,
            headers,
            body: Ok(body.into()),
        }
    }

    /// A transport-level failure instead of a response.
    pub fn err(error: TransportError) -> Self {
        Self {
            status: 0,
            headers: Vec::new(),
            body: Err(error),
        }
    }

    /// A `200` reply whose body is a recorded fixture.
    pub fn json_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let body = std::fs::read_to_string(path)?;
        Ok(Self::ok(200, body))
    }
}

/// A transport that answers from a script and records every request it saw.
///
/// This is the transport the whole test suite uses; nothing in it touches the network. Running out
/// of scripted replies is an [`TransportError::Io`], so a test that under-scripts its transport
/// fails with a message instead of panicking.
#[derive(Debug, Default)]
pub struct StubTransport {
    replies: Mutex<VecDeque<StubReply>>,
    calls: Mutex<Vec<HttpRequest>>,
}

impl StubTransport {
    /// A transport that answers `replies` in order.
    pub fn new(replies: Vec<StubReply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Answers `reply` after the scripted ones.
    pub fn push(&self, reply: StubReply) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Every request `execute` has seen, in order.
    pub fn calls(&self) -> Vec<HttpRequest> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for StubTransport {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| StubReply::err(TransportError::Io("no reply left".to_owned())));
        let url = request.full_url();
        match reply.body {
            Ok(body) => Ok(HttpResponse {
                status: reply.status,
                headers: reply.headers,
                body: body.into_bytes(),
                url,
            }),
            Err(error) => Err(error),
        }
    }
}

/// The retrying front end every caller uses.
pub struct HttpClient {
    transport: Box<dyn Transport>,
    attempts: u32,
    clock: Arc<dyn Clock>,
    verbose: u8,
}

impl HttpClient {
    /// Wraps `transport` with the retry policy: `min(1 + retries, 3)` attempts, waits requested
    /// from `clock`.
    pub fn new(
        transport: Box<dyn Transport>,
        retries: u32,
        clock: Arc<dyn Clock>,
        verbose: u8,
    ) -> Self {
        Self {
            transport,
            attempts: (1 + retries).clamp(1, MAX_ATTEMPTS),
            clock,
            verbose,
        }
    }

    /// Sends `request`, retrying transient failures.
    ///
    /// Retried: transport timeouts, refused/reset connections, DNS failures, plus the `408`, `429`
    /// and `5xx` statuses. Everything else is returned at once. A `Retry-After` header on `429`
    /// or `503` replaces the exponential wait, clamped to a minute; every other retried status
    /// uses the exponential schedule, so a `500` carrying `Retry-After: 120` does not sleep.
    pub fn send(&self, request: &HttpRequest) -> Result<HttpResponse> {
        let mut attempt = 1;
        loop {
            match self.transport.execute(request) {
                Ok(response) => {
                    if retryable_status(response.status()) {
                        if attempt >= self.attempts {
                            return Err(upstream_error(&response));
                        }
                        let delay = retry_delay(&response, self.clock.now(), attempt);
                        self.log(
                            request,
                            attempt,
                            &format!("HTTP {}", response.status()),
                            delay,
                        );
                        self.clock.sleep(delay);
                        attempt += 1;
                    } else if (200..300).contains(&response.status()) {
                        if let Some(error) = undecodable_encoding(&response) {
                            return Err(error);
                        }
                        return Ok(response);
                    } else {
                        return Err(upstream_error(&response));
                    }
                }
                Err(error) if error.is_retryable() && attempt < self.attempts => {
                    let delay = backoff(attempt);
                    self.log(request, attempt, &error.to_string(), delay);
                    self.clock.sleep(delay);
                    attempt += 1;
                }
                Err(error) => return Err(network_error(request, Some(attempt), &error)),
            }
        }
    }

    /// Sends `request` exactly once: no retry, no backoff wait.
    ///
    /// The Nominatim client uses this because its one-request-per-second throttle is taken once
    /// per call, and the retry loop inside [`HttpClient::send`] would put two requests on the wire
    /// 500 ms apart — below the interval the service's usage policy allows. Status handling is the
    /// same as `send`'s last attempt.
    pub fn send_once(&self, request: &HttpRequest) -> Result<HttpResponse> {
        match self.transport.execute(request) {
            Ok(response) => {
                if (200..300).contains(&response.status()) {
                    match undecodable_encoding(&response) {
                        Some(error) => Err(error),
                        None => Ok(response),
                    }
                } else {
                    Err(upstream_error(&response))
                }
            }
            Err(error) => Err(network_error(request, None, &error)),
        }
    }

    /// The transport, for tests that assert what was sent.
    #[must_use]
    pub fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }

    /// One `-vv` line per retry, on stderr: request URL (redacted), attempt, reason, delay.
    fn log(&self, request: &HttpRequest, attempt: u32, reason: &str, delay: Duration) {
        if self.verbose > 1 {
            eprintln!(
                "http: {} {} attempt {}/{} after {reason}; sleeping {:.1} s",
                request.method().as_str(),
                request.redacted_url(),
                attempt + 1,
                self.attempts,
                delay.as_secs_f64()
            );
        }
    }
}

/// Maps an exhausted transport failure: retryable ones name the attempt count when there was more
/// than one attempt, permanent ones and single attempts do not.
fn network_error(request: &HttpRequest, attempts: Option<u32>, error: &TransportError) -> Error {
    let method = request.method().as_str();
    let url = request.redacted_url();
    match (error.is_retryable(), attempts) {
        (true, Some(attempts)) => Error::Network(format!(
            "{method} {url} failed after {attempts} attempts: {error}"
        )),
        _ => Error::Network(format!("{method} {url} failed: {error}")),
    }
}

/// Whether a status is worth another attempt.
fn retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

/// `0.5 s`, `1 s`, `2 s`, … — one step further for each attempt already spent.
fn backoff(attempt: u32) -> Duration {
    BACKOFF_STEP * 2_u32.pow(attempt.saturating_sub(1))
}

/// How long to wait before retrying `response`: its `Retry-After` on `429`/`503` (clamped to a
/// minute), the exponential schedule otherwise.
///
/// The doc for [`HttpClient::send`] scopes `Retry-After` to those two statuses; consulting it on
/// every retried status would let a `500` carrying `Retry-After: 120` sleep 60 s per attempt.
fn retry_delay(response: &HttpResponse, now: SystemTime, attempt: u32) -> Duration {
    if matches!(response.status(), 429 | 503) {
        response
            .retry_after(now)
            .map_or_else(|| backoff(attempt), |after| after.min(MAX_RETRY_AFTER))
    } else {
        backoff(attempt)
    }
}

/// The error for a `2xx` response whose `Content-Encoding` this build cannot decode, or `None`
/// when the coding is absent or `identity`.
///
/// `ureq` is built without its transparent content decoders (see `Cargo.toml`), so a body that
/// still declares a coding arrives as its still-compressed wire bytes. Refusing it names the cap
/// that request was read under instead of handing the parsers something they would misread.
fn undecodable_encoding(response: &HttpResponse) -> Option<Error> {
    let encoding = response.header("content-encoding")?.trim();
    if encoding.is_empty() || encoding.eq_ignore_ascii_case("identity") {
        return None;
    }
    Some(Error::Upstream {
        provider: host_of(&response.url),
        status: Some(response.status()),
        message: format!(
            "the response is `{encoding}`-encoded and this build does not decode content codings; \
             the {MAX_BODY_BYTES}-byte body cap bounds the wire bytes, not a decoded expansion"
        ),
    })
}

/// The upstream's own words when it sends an error envelope, the body's first 200 characters
/// otherwise.
///
/// Both paths are bounded: an upstream is free to answer a 401 with a `reason` field that fills
/// the whole [`MAX_BODY_BYTES`] budget, and that text ends up in an [`Error::Upstream`] message
/// (and, for a fallback chain, in every attempt row of the [`Error::Chain`] report).
fn error_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let field = |name: &str| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        };
        if let Some(reason) = field("reason") {
            return first_200(reason);
        }
        if value.get("error").and_then(serde_json::Value::as_bool) == Some(true)
            && let Some(message) = field("message")
        {
            return first_200(message);
        }
    }
    first_200(body)
}

/// The first 200 characters of `text`.
fn first_200(text: &str) -> String {
    text.chars().take(200).collect()
}

/// A non-2xx response the client is not going to retry.
fn upstream_error(response: &HttpResponse) -> Error {
    let message = if response.body().trim_start().starts_with('<') {
        // SMHI (and others) answer a 404 with an HTML error page; pasting tags into the message
        // helps nobody, so the shape is named instead.
        "the upstream returned an HTML error page".to_owned()
    } else {
        error_message(response.body())
    };
    Error::Upstream {
        provider: host_of(&response.url),
        status: Some(response.status),
        message,
    }
}

/// The host of a URL, for error messages and for the `provider` field of [`Error::Upstream`].
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?']).next().unwrap_or(rest);
    if host.is_empty() {
        url.to_owned()
    } else {
        host.to_owned()
    }
}

/// Percent-encodes one query name or value.
fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_SET).to_string()
}

/// Translates a `ureq` failure into the crate's transport taxonomy.
fn ureq_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        ureq::Error::HostNotFound => TransportError::Dns("host not found".to_owned()),
        ureq::Error::ConnectionFailed => {
            TransportError::Connect("no connection could be made".to_owned())
        }
        ureq::Error::Tls(detail) => TransportError::Tls(detail.to_owned()),
        ureq::Error::BodyExceedsLimit(_) => TransportError::TooLarge,
        ureq::Error::Io(error) => match error.kind() {
            std::io::ErrorKind::TimedOut => TransportError::Timeout,
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => TransportError::Reset,
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::AddrNotAvailable => TransportError::Connect(error.to_string()),
            _ => TransportError::Io(error.to_string()),
        },
        other => TransportError::Io(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use super::{
        HttpClient, HttpRequest, HttpResponse, Method, StubReply, StubTransport, TransportError,
        error_message, follows_redirects, host_of, retry_delay, undecodable_encoding,
    };
    use crate::cache::{Clock, FakeClock};

    /// A response with the given status and headers, for header-parsing tests.
    fn response(headers: Vec<(&str, &str)>) -> HttpResponse {
        HttpResponse {
            status: 429,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            body: Vec::new(),
            url: "https://example.invalid/answered".to_owned(),
        }
    }

    #[test]
    fn only_a_header_free_request_may_follow_a_redirect() {
        assert!(
            follows_redirects(&HttpRequest::get("https://example.invalid/a")),
            "a public request must follow the FPAS canonical redirect"
        );
        let credentialed = HttpRequest::get("https://example.invalid/a")
            .header("X-QW-Api-Key", "0123456789abcdef")
            .secret("0123456789abcdef");
        assert!(
            !follows_redirects(&credentialed),
            "a credential header must not survive a hop"
        );
    }

    #[test]
    fn normalized_sorts_and_encodes_the_request() {
        let request = HttpRequest::get("https://geocoding-api.open-meteo.com/v1/search")
            .query("name", "Beijing, CN")
            .query("count", "10")
            .header("User-Agent", "cirrocast/0.1.0");
        assert_eq!(
            request.normalized(),
            "GET https://geocoding-api.open-meteo.com/v1/search?count=10&name=Beijing%2C%20CN\nuser-agent: cirrocast/0.1.0"
        );
        let reordered = HttpRequest::get("https://geocoding-api.open-meteo.com/v1/search")
            .query("count", "10")
            .query("name", "Beijing, CN")
            .header("user-agent", "cirrocast/0.1.0");
        assert_eq!(request.normalized(), reordered.normalized());
    }

    #[test]
    fn a_secret_is_redacted_everywhere_the_request_is_printed() {
        let request = HttpRequest::get("https://api.example.invalid/data/2.5/weather")
            .query("lat", "39.9042")
            .query("appid", "sk-live-0123456789abcdef")
            .header("X-Api-Key", "sk-live-0123456789abcdef")
            .secret("sk-live-0123456789abcdef");

        assert!(request.full_url().contains("sk-live-0123456789abcdef"));
        assert!(!request.redacted_url().contains("sk-live-0123456789abcdef"));
        assert!(request.redacted_url().contains("appid=***"));
        assert!(
            !request
                .redacted_normalized()
                .contains("sk-live-0123456789abcdef")
        );
        assert!(request.redacted_normalized().contains("***"));

        let debug = format!("{request:?}");
        assert!(!debug.contains("sk-live-0123456789abcdef"), "{debug}");
    }

    #[test]
    fn redaction_takes_the_longest_secret_first() {
        let request = HttpRequest::get("https://api.example.invalid/v1/forecast")
            .query("key", "abcdef")
            .secret("abcd")
            .secret("abcdef");
        assert_eq!(
            request.redacted_url(),
            "https://api.example.invalid/v1/forecast?key=***"
        );
    }

    #[test]
    fn a_secret_outside_the_unreserved_set_is_redacted_encoded_too() {
        // The wire spelling of a query value is percent-encoded; a key with `+` or `/` would slip
        // past a redaction that only looked for the raw text.
        let secret = "abc+def/SECRET=123";
        let request = HttpRequest::get("https://api.example.invalid/v1/forecast")
            .query("key", secret)
            .secret(secret);

        let printed = request.redacted_url();
        assert!(!printed.contains(secret), "{printed}");
        assert!(!printed.contains("abc%2Bdef%2FSECRET%3D123"), "{printed}");
        assert!(printed.contains("key=***"), "{printed}");
        assert!(
            !request.redacted_normalized().contains(secret),
            "the cache-envelope spelling must be redacted too"
        );
    }

    #[test]
    fn a_json_error_reason_is_bounded_like_a_raw_body() {
        let reason = "x".repeat(9_000);
        let body = format!(r#"{{"error":true,"reason":"{reason}"}}"#);
        assert_eq!(error_message(&body).chars().count(), 200);
    }

    #[test]
    fn full_url_keeps_the_insertion_order() {
        let request = HttpRequest::get("https://example.invalid/search")
            .query("q", "Tsinghua")
            .query("limit", "10");
        assert_eq!(
            request.full_url(),
            "https://example.invalid/search?q=Tsinghua&limit=10"
        );
        assert_eq!(
            HttpRequest::get("https://example.invalid/").full_url(),
            "https://example.invalid/"
        );
        assert_eq!(request.method(), Method::Get);
    }

    #[test]
    fn retry_after_reads_seconds_dates_and_rejects_junk() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(
            response(vec![("Retry-After", "7")]).retry_after(now),
            Some(Duration::from_secs(7))
        );
        assert_eq!(response(vec![("Retry-After", "-3")]).retry_after(now), None);
        assert_eq!(
            response(vec![("Retry-After", "soon")]).retry_after(now),
            None
        );
        assert_eq!(response(vec![]).retry_after(now), None);

        let past = chrono::DateTime::<chrono::Utc>::from(now - Duration::from_secs(60));
        assert_eq!(
            response(vec![("Retry-After", &past.to_rfc2822())]).retry_after(now),
            None
        );
        let future = chrono::DateTime::<chrono::Utc>::from(now + Duration::from_secs(30));
        assert_eq!(
            response(vec![("Retry-After", &future.to_rfc2822())]).retry_after(now),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn upstream_messages_prefer_the_reason_envelope() {
        assert_eq!(
            error_message(
                r#"{"error":true,"reason":"Parameter 'name' must be at least 2 characters."}"#
            ),
            "Parameter 'name' must be at least 2 characters."
        );
        assert_eq!(
            error_message(r#"{"error":true,"reason":"","message":"quota exceeded"}"#),
            "quota exceeded"
        );
        let long = "x".repeat(300);
        assert_eq!(error_message(&long).chars().count(), 200);
        assert_eq!(error_message("<html>oops</html>"), "<html>oops</html>");
    }

    #[test]
    fn hosts_label_upstream_errors() {
        assert_eq!(host_of("https://ipwho.is/?x=1"), "ipwho.is");
        assert_eq!(
            host_of("https://geocoding-api.open-meteo.com/v1/search"),
            "geocoding-api.open-meteo.com"
        );
        assert_eq!(host_of("not-a-url"), "not-a-url");
    }

    #[test]
    fn a_configured_proxy_wins_over_the_environment() {
        let configured = crate::config::Network {
            proxy: "http://127.0.0.1:8080".to_owned(),
            ..crate::config::Network::default()
        };
        let Some(proxy) = super::resolve_proxy(&configured).expect("a valid proxy URL") else {
            panic!("a configured proxy must be used");
        };
        assert_eq!(proxy.host(), "127.0.0.1");
        assert_eq!(proxy.port(), 8080);
        assert!(!proxy.is_from_env());

        let broken = crate::config::Network {
            proxy: "://nope".to_owned(),
            ..crate::config::Network::default()
        };
        assert!(super::resolve_proxy(&broken).is_err());
    }

    #[test]
    fn transport_errors_retry_only_transient_conditions() {
        assert!(TransportError::Timeout.is_retryable());
        assert!(TransportError::Connect("refused".into()).is_retryable());
        assert!(TransportError::Reset.is_retryable());
        assert!(TransportError::Dns("nx".into()).is_retryable());
        assert!(!TransportError::Tls("bad cert".into()).is_retryable());
        assert!(!TransportError::Io("frame".into()).is_retryable());
        assert!(
            !TransportError::Blocked.is_retryable() && !TransportError::TooLarge.is_retryable(),
            "the guard and the body cap fail the same way on every attempt"
        );
    }

    #[test]
    fn a_body_over_the_cap_is_a_transport_failure() {
        // The limit is installed on the body reader; what is testable without a server is the
        // mapping: ureq's `BodyExceedsLimit` becomes our own non-retryable variant, which the
        // client reports as a network failure (exit 3) instead of allocating without bound.
        let error = super::ureq_error(ureq::Error::BodyExceedsLimit(super::MAX_BODY_BYTES));
        assert!(matches!(error, TransportError::TooLarge));
        assert!(!error.is_retryable());
        assert!(
            error.to_string().contains("byte cap"),
            "the message must name the cap: {error}"
        );
    }

    #[test]
    fn only_loopback_targets_survive_the_network_guard() {
        for url in [
            "http://127.0.0.1:8080/v1/forecast",
            "http://127.5.6.7/",
            "http://localhost:1234/x?y=1",
            "http://[::1]:8080/v1/forecast",
            "https://LOCALHOST/x",
            // Userinfo is stripped before the host is read, so a loopback host stays loopback.
            "http://user:password@localhost:8080/x",
            "http://user@[::1]:8080/x",
        ] {
            assert!(
                super::is_loopback_url(url),
                "{url} is the local machine and stays reachable"
            );
        }
        for url in [
            "https://api.open-meteo.com/v1/forecast",
            "https://127.0.0.1.example.com/v1",
            "http://10.0.0.1/",
            "ftp://example.invalid/x",
            "not-a-url",
            // The userinfo must not mask the real host: `localhost:8080` here is userinfo.
            "http://localhost:8080@evil.com/",
            "http://[::1]@evil.com/x",
        ] {
            assert!(
                !super::is_loopback_url(url),
                "{url} needs a socket and must be blocked"
            );
        }
    }

    #[test]
    fn retry_after_only_extends_a_429_or_503() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let with_header = |status: u16| HttpResponse {
            status,
            headers: vec![("Retry-After".to_owned(), "120".to_owned())],
            body: Vec::new(),
            url: "https://example.invalid/x".to_owned(),
        };
        // Only 429/503 consult the header (clamped to a minute); a 500 keeps the backoff schedule.
        assert_eq!(
            retry_delay(&with_header(429), now, 1),
            Duration::from_secs(60)
        );
        assert_eq!(
            retry_delay(&with_header(503), now, 1),
            Duration::from_secs(60)
        );
        assert_eq!(
            retry_delay(&with_header(500), now, 1),
            Duration::from_millis(500)
        );
        assert_eq!(
            retry_delay(&with_header(502), now, 2),
            Duration::from_millis(1_000)
        );
    }

    #[test]
    fn a_non_identity_content_encoding_is_refused_as_an_upstream_error() {
        // The stub transport is what lets this be tested without a server: it answers with a
        // `Content-Encoding` header the real transport would have handed back undecoded.
        let gzip = HttpResponse {
            status: 200,
            headers: vec![("Content-Encoding".to_owned(), "gzip".to_owned())],
            body: b"not really gzip".to_vec(),
            url: "https://example.invalid/data".to_owned(),
        };
        let error = undecodable_encoding(&gzip).expect("a gzip response is refused");
        match error {
            crate::error::Error::Upstream {
                provider, message, ..
            } => {
                assert_eq!(provider, "example.invalid");
                assert!(message.contains("gzip"), "{message}");
                assert!(
                    message.contains(&super::MAX_BODY_BYTES.to_string()),
                    "the message must name the cap: {message}"
                );
            }
            other => panic!("expected an upstream error, got {other:?}"),
        }
        // `identity` and an absent header are both fine.
        let identity = HttpResponse {
            headers: vec![("Content-Encoding".to_owned(), "identity".to_owned())],
            ..gzip
        };
        assert!(undecodable_encoding(&identity).is_none());
    }

    #[test]
    fn socks_protocols_are_recognised() {
        assert!(super::is_socks(ureq::ProxyProtocol::Socks4));
        assert!(super::is_socks(ureq::ProxyProtocol::Socks4A));
        assert!(super::is_socks(ureq::ProxyProtocol::Socks5));
        assert!(super::is_socks(ureq::ProxyProtocol::Socks5h));
        assert!(!super::is_socks(ureq::ProxyProtocol::Http));
        assert!(!super::is_socks(ureq::ProxyProtocol::Https));
    }

    #[test]
    fn the_client_refuses_an_undecodable_body_through_the_stub_transport() {
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
        let transport = StubTransport::new(vec![StubReply::status(
            200,
            vec![("Content-Encoding".to_owned(), "br".to_owned())],
            "compressed",
        )]);
        let client = HttpClient::new(Box::new(transport), 0, clock, 0);
        let error = client
            .send(&HttpRequest::get("https://example.invalid/data"))
            .expect_err("a brotli body is refused");
        assert!(
            matches!(error, crate::error::Error::Upstream { .. }),
            "exit-3 upstream error, got {error:?}"
        );
    }
}
