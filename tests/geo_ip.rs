// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The IP location chain, driven by recorded answers and a scripted transport.
//!
//! Nothing here opens a socket: every reply is a fixture under `tests/fixtures/ip/` (minimised
//! recordings, with the machine's own address replaced by an RFC 5737 documentation address), and
//! every clock is a [`FakeClock`], so cache expiry is asserted instead of waited for.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{TimeZone as _, Utc};
use chrono_tz::Tz;
use cirrocast::cache::{Cache, CacheKey, CacheMode, Clock, FakeClock};
use cirrocast::error::Error;
use cirrocast::geo::ip::{IpLocator, IpLocatorChain, IpService};
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::LocationSource;

/// The TTL the CLI passes: `cache.ip_ttl_secs`, one day.
const TTL: Duration = Duration::from_hours(24);

/// A fixed instant — 2026-01-01T00:00:00Z — so freshness never depends on the wall clock.
fn fixed_now() -> SystemTime {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap().into()
}

/// The path of the recorded answer `tests/fixtures/ip/<name>`.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ip")
        .join(name)
}

/// A recorded answer as the reply a stub hands back for one request.
fn reply(name: &str) -> StubReply {
    let path = fixture(name);
    StubReply::json_file(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// A stub transport, a fake clock and a throwaway cache, wired into one client.
struct Harness {
    _temp: tempfile::TempDir,
    transport: Arc<StubTransport>,
    clock: Arc<FakeClock>,
    cache: Cache,
    http: HttpClient,
}

impl Harness {
    /// The harness the module describes: normal caching, three attempts, no verbosity.
    fn new(replies: Vec<StubReply>) -> Self {
        Self::with_mode(replies, CacheMode::Normal)
    }

    /// The same harness with another cache mode.
    fn with_mode(replies: Vec<StubReply>, mode: CacheMode) -> Self {
        let temp = tempfile::tempdir().expect("a temporary cache root");
        let transport = Arc::new(StubTransport::new(replies));
        let clock = Arc::new(FakeClock::new(fixed_now()));
        let shared: Arc<dyn Clock> = clock.clone();
        let cache = Cache::with_root(temp.path(), mode, Arc::clone(&shared), 0);
        let http = HttpClient::new(Box::new(Arc::clone(&transport)), 3, shared, 0);
        Self {
            _temp: temp,
            transport,
            clock,
            cache,
            http,
        }
    }

    /// The chain under test, trying `services` in the order given.
    fn chain(&self, services: Vec<IpService>) -> IpLocatorChain<'_> {
        IpLocatorChain::new(&self.http, &self.cache, services, TTL)
    }

    /// The chain `IpService::chain("auto")` selects.
    fn auto(&self) -> IpLocatorChain<'_> {
        self.chain(IpService::chain("auto").expect("`auto` is a known setting"))
    }

    /// How many requests the transport has seen.
    fn calls(&self) -> usize {
        self.transport.calls().len()
    }
}

/// The setting to service list mapping, including the rejections.
#[test]
fn the_setting_selects_the_services() {
    assert_eq!(
        IpService::chain("auto").expect("`auto` is a known setting"),
        vec![IpService::IpWhoIs, IpService::IpApiCo]
    );
    assert_eq!(
        IpService::chain("ipwhois").expect("`ipwhois` is a known setting"),
        vec![IpService::IpWhoIs]
    );
    assert_eq!(
        IpService::chain("ipapi").expect("`ipapi` is a known setting"),
        vec![IpService::IpApiCo]
    );

    for junk in ["", " ", "ipwho.is", "ip-api", "Auto", "both", "all"] {
        let error = IpService::chain(junk).expect_err("junk must be rejected");
        assert!(
            matches!(error, Error::Usage(_)),
            "`{junk}` should be a usage error, got {error}"
        );
        assert_eq!(error.exit_code(), 2, "`{junk}` should exit 2");
    }

    assert_eq!(IpService::IpWhoIs.slug(), "ipwho-is");
    assert_eq!(IpService::IpWhoIs.label(), "ipwho.is");
    assert_eq!(IpService::IpApiCo.slug(), "ipapi-co");
    assert_eq!(IpService::IpApiCo.label(), "ipapi.co");
}

/// The primary's answer, its request and the service the disclosure line reports.
#[test]
#[allow(clippy::float_cmp)]
fn an_ipwho_is_answer_becomes_the_location() {
    let harness = Harness::new(vec![reply("ipwho_is_beijing.json")]);
    let chain = harness.auto();

    let (location, service) = chain.locate_with_service().expect("the fixture resolves");
    assert_eq!(service, IpService::IpWhoIs);
    assert_eq!(location.name, "Beijing");
    assert_eq!(location.admin1.as_deref(), Some("Beijing"));
    assert_eq!(location.country, "China");
    assert_eq!(location.country_code.as_deref(), Some("CN"));
    assert_eq!(location.lat, 39.907_503);
    assert_eq!(location.lon, 116.397_228);
    assert_eq!(location.tz, Tz::Asia__Shanghai);
    assert_eq!(location.source, LocationSource::Ip);
    assert_eq!(location.elevation_m, None);
    assert_eq!(location.population, None);

    // The trait method returns the same location; the second lookup comes from the cache.
    assert_eq!(chain.locate().expect("the answer is cached"), location);
    assert_eq!(harness.calls(), 1);

    let calls = harness.transport.calls();
    let request = calls.first().expect("one request");
    assert_eq!(request.normalized(), "GET https://ipwho.is/");
}

/// A refused address falls through, and the fallback's country fields are read the right way round.
#[test]
fn a_refused_address_falls_through_to_the_next_service() {
    let harness = Harness::new(vec![
        reply("ipwho_is_failure.json"),
        reply("ipapi_co_beijing.json"),
    ]);
    let (location, service) = harness
        .auto()
        .locate_with_service()
        .expect("the fallback resolves");

    assert_eq!(service, IpService::IpApiCo);
    assert_eq!(location.name, "Beijing");
    assert_eq!(location.admin1.as_deref(), Some("Beijing"));
    // In an ipapi.co answer `country` is the two-letter code and `country_name` the display name.
    assert_eq!(location.country, "China");
    assert_eq!(location.country_code.as_deref(), Some("CN"));
    assert_eq!(location.tz, Tz::Asia__Shanghai);
    assert_eq!(location.source, LocationSource::Ip);

    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].normalized(), "GET https://ipwho.is/");
    assert_eq!(calls[1].normalized(), "GET https://ipapi.co/json/");
    // Each service owns its own entry, so a fallback answer is never served as the primary's.
    assert!(
        harness
            .cache
            .entry_path(&CacheKey::ip("ipapi-co"))
            .is_file()
    );
}

/// With both services refusing, the failure names every attempt instead of only the last one.
#[test]
fn both_services_failing_names_every_attempt() {
    let harness = Harness::new(vec![
        reply("ipwho_is_failure.json"),
        reply("ipapi_co_error.json"),
    ]);
    let error = harness.auto().locate().expect_err("both services refuse");

    assert!(matches!(error, Error::Chain { .. }), "{error}");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(
        text.starts_with("all IP location services failed:"),
        "{text}"
    );
    assert!(text.contains("ipwho.is"), "{text}");
    assert!(text.contains("ipapi.co"), "{text}");
    // The short `reason` wins over the longer `message` sentence.
    assert!(text.contains("RateLimited"), "{text}");
    assert!(!text.contains("ratelimited/"), "{text}");
    assert_eq!(harness.calls(), 2);
}

/// A fresh answer is served from the cache; an expired one is fetched again.
#[test]
fn a_cached_answer_is_served_until_it_expires() {
    let harness = Harness::new(vec![reply("ipwho_is_beijing.json")]);
    let chain = harness.auto();
    chain.locate().expect("the first lookup fetches");
    assert_eq!(harness.calls(), 1);
    assert!(
        harness
            .cache
            .entry_path(&CacheKey::ip("ipwho-is"))
            .is_file()
    );

    // The script is empty now, so a second lookup can only succeed from the cache.
    let cached = chain
        .locate()
        .expect("the second lookup is served from the cache");
    assert_eq!(cached.name, "Beijing");
    assert_eq!(harness.calls(), 1);

    harness.clock.advance(Duration::from_secs(86_401));
    harness.transport.push(reply("ipwho_is_beijing.json"));
    chain.locate().expect("the expired entry is fetched again");
    assert_eq!(harness.calls(), 2);
}

/// A zone the parser does not know is an error, never a silent `UTC`.
#[test]
fn an_unusable_timezone_names_the_zone_and_the_service() {
    let harness = Harness::new(vec![reply("ipwho_is_bad_timezone.json")]);
    let chain = harness.chain(IpService::chain("ipwhois").expect("`ipwhois` is a known setting"));
    let error = chain
        .locate()
        .expect_err("`Mars/Olympus` is not an IANA zone");

    assert!(matches!(error, Error::Chain { .. }), "{error}");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("Mars/Olympus"), "{text}");
    assert!(text.contains("ipwho.is"), "{text}");
    assert_eq!(harness.calls(), 1);
}

/// A primary that keeps answering `5xx` is retried, then falls through to the fallback.
#[test]
fn a_failing_primary_falls_through_to_the_fallback() {
    let mut replies = vec![StubReply::ok(503, "unavailable"); 3];
    replies.push(reply("ipapi_co_beijing.json"));
    let harness = Harness::new(replies);

    let (location, service) = harness
        .auto()
        .locate_with_service()
        .expect("the fallback resolves");
    assert_eq!(service, IpService::IpApiCo);
    assert_eq!(location.name, "Beijing");

    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 4, "three attempts plus the fallback");
    assert_eq!(calls[3].normalized(), "GET https://ipapi.co/json/");
    // The two waits between the attempts went to the injected clock.
    assert_eq!(harness.clock.sleeps().len(), 2);
}

/// A required field the answer omits names the service and the field, instead of becoming a zero.
#[test]
fn a_missing_required_field_names_the_service_and_the_field() {
    let cases = [
        (
            r#"{"success":true,"region":"Beijing","country":"China","latitude":39.9,"longitude":116.4,"timezone":{"id":"Asia/Shanghai"}}"#,
            "city",
        ),
        (
            r#"{"success":true,"city":"Beijing","country":"China","longitude":116.4,"timezone":{"id":"Asia/Shanghai"}}"#,
            "latitude",
        ),
        (
            r#"{"success":true,"city":"Beijing","country":"China","latitude":39.9,"timezone":{"id":"Asia/Shanghai"}}"#,
            "longitude",
        ),
        (
            r#"{"success":true,"city":"Beijing","country":"China","latitude":39.9,"longitude":116.4}"#,
            "timezone",
        ),
    ];

    for (body, field) in cases {
        // No caching: every case must reach the transport with its own body.
        let harness = Harness::with_mode(vec![StubReply::ok(200, body)], CacheMode::NoCache);
        let chain = harness.chain(IpService::chain("ipwhois").expect("`ipwhois` is known"));
        let error = chain.locate().expect_err("a missing field is a failure");

        assert!(matches!(error, Error::Chain { .. }), "{error}");
        assert_eq!(error.exit_code(), 3);
        let text = error.to_string();
        assert!(text.contains("ipwho.is"), "{text}");
        assert!(text.contains(field), "`{body}` should name {field}: {text}");
        assert_eq!(harness.calls(), 1);
    }
}

/// An empty chain never asks anyone and says why.
#[test]
fn an_empty_chain_makes_no_request() {
    let harness = Harness::new(vec![reply("ipwho_is_beijing.json")]);
    let error = harness
        .chain(Vec::new())
        .locate()
        .expect_err("there is no service to ask");

    assert!(matches!(error, Error::Usage(_)), "{error}");
    assert_eq!(error.exit_code(), 2);
    assert_eq!(harness.calls(), 0);
}
