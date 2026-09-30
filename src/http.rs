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
//!   wait is requested from an injected [`Clock`](crate::cache::Clock), so a test asserts the
//!   schedule instead of sleeping through it;
//! * `http_status_as_error(false)` keeps the status *and* the body of a 4xx/5xx response, which is
//!   what lets the policy retry `429`/`5xx` and report the upstream's own `reason` text;
//! * [`HttpRequest::normalized`] is the cache key input: query pairs sorted and encoded, header
//!   names lower-cased, so two spellings of the same request hash to one cache entry.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    method: Method,
    url: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
}

impl HttpRequest {
    /// A `GET` for `url`, without a query string or extra headers.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            query: Vec::new(),
            headers: Vec::new(),
            timeout: None,
        }
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

    /// Overrides the client's timeout for this request only.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
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

    /// The per-request timeout override, when the caller set one.
    #[must_use]
    pub const fn timeout_duration(&self) -> Option<Duration> {
        self.timeout
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    url: String,
}

impl HttpResponse {
    /// The status code.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// The body, as text (the only wire formats this project consumes are JSON and plain text).
    #[must_use]
    pub fn body(&self) -> &str {
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
        serde_json::from_str(&self.body).map_err(|error| Error::Upstream {
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
    /// Any other I/O or protocol failure.
    #[error("I/O failure: {0}")]
    Io(String),
}

impl TransportError {
    /// Whether trying again can plausibly succeed.
    ///
    /// Transient conditions retry; a broken TLS setup or a protocol error does not, because the
    /// next attempt would fail identically.
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
fn resolve_proxy(config: &Network) -> Result<Option<ureq::Proxy>> {
    if config.proxy.trim().is_empty() {
        return Ok(ureq::Proxy::try_from_env());
    }
    let url = config.proxy.trim();
    ureq::Proxy::new(url).map(Some).map_err(|error| {
        Error::Config(format!("network.proxy `{url}` is not a proxy URL: {error}"))
    })
}

/// Shared transports: an `Arc` around one is itself a transport, which is what lets a test keep a
/// handle on the [`StubTransport`] it handed to the client.
impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        (**self).execute(request)
    }
}

/// The real transport: `ureq` over rustls, with the agent configured once.
pub struct UreqTransport {
    agent: Agent,
}

impl UreqTransport {
    /// Builds the agent: statuses are *not* errors (the retry policy needs the body), the user
    /// agent is [`UA`], all three phase timeouts use `timeout`, and the proxy comes from
    /// `[network] proxy` or, when that is empty, from the `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY`
    /// environment.
    pub fn new(config: &Network, timeout: Duration) -> Result<Self> {
        let proxy = resolve_proxy(config)?;

        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(UA)
            .timeout_connect(Some(timeout))
            .timeout_recv_response(Some(timeout))
            .timeout_recv_body(Some(timeout))
            .proxy(proxy)
            .build()
            .new_agent();
        Ok(Self { agent })
    }
}

impl Transport for UreqTransport {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let mut builder = match request.method() {
            Method::Get => self.agent.get(request.full_url()),
        };
        for (name, value) in request.headers() {
            builder = builder.header(name, value);
        }
        if let Some(timeout) = request.timeout_duration() {
            builder = builder.config().timeout_global(Some(timeout)).build();
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
        let body = response.body_mut().read_to_string().map_err(ureq_error)?;
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
                body,
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
    /// or `503` replaces the exponential wait, clamped to a minute.
    pub fn send(&self, request: &HttpRequest) -> Result<HttpResponse> {
        let mut attempt = 1;
        loop {
            match self.transport.execute(request) {
                Ok(response) => {
                    if retryable_status(response.status()) {
                        if attempt >= self.attempts {
                            return Err(upstream_error(&response));
                        }
                        let delay = response
                            .retry_after(self.clock.now())
                            .map_or_else(|| backoff(attempt), |after| after.min(MAX_RETRY_AFTER));
                        self.log(
                            request,
                            attempt,
                            &format!("HTTP {}", response.status()),
                            delay,
                        );
                        self.clock.sleep(delay);
                        attempt += 1;
                    } else if (200..300).contains(&response.status()) {
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
                Err(error) => return Err(network_error(request, attempt, &error)),
            }
        }
    }

    /// The transport, for tests that assert what was sent.
    #[must_use]
    pub fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }

    /// One `-v` line per retry, on stderr.
    fn log(&self, request: &HttpRequest, attempt: u32, reason: &str, delay: Duration) {
        if self.verbose > 0 {
            eprintln!(
                "http: {} {} attempt {}/{} after {reason}; sleeping {:.1} s",
                request.method().as_str(),
                request.full_url(),
                attempt + 1,
                self.attempts,
                delay.as_secs_f64()
            );
        }
    }
}

/// Maps an exhausted transport failure: retryable ones name the attempt count, permanent ones do
/// not.
fn network_error(request: &HttpRequest, attempts: u32, error: &TransportError) -> Error {
    let method = request.method().as_str();
    let url = request.full_url();
    if error.is_retryable() {
        Error::Network(format!(
            "{method} {url} failed after {attempts} attempts: {error}"
        ))
    } else {
        Error::Network(format!("{method} {url} failed: {error}"))
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

/// The upstream's own words when it sends an error envelope, the body's first 200 characters
/// otherwise.
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
            return reason.to_owned();
        }
        if value.get("error").and_then(serde_json::Value::as_bool) == Some(true)
            && let Some(message) = field("message")
        {
            return message.to_owned();
        }
    }
    body.chars().take(200).collect()
}

/// A non-2xx response the client is not going to retry.
fn upstream_error(response: &HttpResponse) -> Error {
    Error::Upstream {
        provider: host_of(&response.url),
        status: Some(response.status),
        message: error_message(&response.body),
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
    use std::time::{Duration, SystemTime};

    use super::{HttpRequest, HttpResponse, Method, TransportError, error_message, host_of};

    /// A response with the given status and headers, for header-parsing tests.
    fn response(headers: Vec<(&str, &str)>) -> HttpResponse {
        HttpResponse {
            status: 429,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            body: String::new(),
            url: "https://example.invalid/answered".to_owned(),
        }
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
        assert_eq!(request.timeout_duration(), None);
        assert_eq!(
            request.timeout(Duration::from_secs(2)).timeout_duration(),
            Some(Duration::from_secs(2))
        );
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
    }
}
