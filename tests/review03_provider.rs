// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Provider-level regression tests for the 2026-10-05 review (§3.1, §3.6, §3.12).
//!
//! Nothing here opens a socket: the `QWeather` and METAR payloads come from `tests/fixtures/`
//! through `StubTransport`, and the cache is a throwaway directory. The §3.1 case goes one step
//! further and makes that directory read-only, so the write the cache performs after a successful
//! fetch fails the way a read-only `$XDG_CACHE_HOME` makes it fail.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use cirrocast::cache::{Cache, CacheMode};
use cirrocast::config::Config;
use cirrocast::config::keys::KeyStore;
use cirrocast::http::{HttpClient, StubTransport};
use cirrocast::model::{Condition, Location};
use cirrocast::paths::Paths;
use cirrocast::provider::metar::Metar;
use cirrocast::provider::qweather::QWeather;
use cirrocast::provider::{Env, FetchRequest, HourlyResolution, Provider};

use common::{ProviderRun, fixture_location, fixture_reply, provider_clock};

/// The `QWeather` key the fixtures were scrubbed of; the tests store a stand-in.
const KEY: &str = "test-key-0123456789abcdef";

/// The account host the `QWeather` tests configure.
const HOST: &str = "https://example.re.qweatherapi.com";

/// A `QWeather` run over `replies`, with the account host and key configured.
fn qweather_run(replies: Vec<cirrocast::http::StubReply>) -> ProviderRun {
    let mut config = Config::default();
    HOST.clone_into(&mut config.providers.qweather.host);
    let run = ProviderRun::with_config(
        replies,
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
        config,
    );
    run.with_key("qweather", KEY);
    run
}

/// The current-condition code one `tests/fixtures/qweather/current-<code>.json` decodes to.
///
/// `days = 0` keeps the fetch to the current endpoint, so one scripted reply is enough and the
/// assertion is about the code mapping alone.
fn decoded_condition(code: &str) -> Condition {
    let run = qweather_run(vec![fixture_reply(
        "qweather",
        &format!("current-{code}.json"),
    )]);
    run.fetch_with(&QWeather, &fixture_location("beijing"), 0)
        .expect("the current fixture parses")
        .current
        .expect("the fixture has current conditions")
        .weather
}

#[test]
fn qweather_307_and_308_are_heavy_rain() {
    // `QWeather`'s own table: 307 大雨 / Heavy Rain, 308 极端降雨 / Extreme Rain. Both are the
    // heavy-rain band (WMO 65); 309 must not be folded into it (see the drizzle case below).
    assert_eq!(decoded_condition("307"), Condition::from_u8(65));
    assert_eq!(decoded_condition("308"), Condition::from_u8(65));
}

#[test]
fn qweather_309_is_drizzle_not_heavy_rain() {
    // 309 毛毛雨/细雨 / Drizzle Rain: the heavy-rain arm used to swallow it because it spans
    // 307..=312, so drizzle rendered as heavy rain with the heavy-rain art block.
    assert_eq!(decoded_condition("309"), Condition::from_u8(53));
}

#[test]
fn qweather_515_is_fog_not_freezing_drizzle() {
    // 515 Extra Heavy Fog is the strongest member of the 500 fog family; review-01's "515 is
    // freezing drizzle" reading was wrong and encoded 56 here.
    assert_eq!(decoded_condition("515"), Condition::from_u8(45));
}

/// A cache rooted at `cache_root`, with `paths` pointing at the surrounding temporary tree.
fn read_only_cache(cache_root: &std::path::Path) -> Cache {
    Cache::with_root(
        cache_root,
        CacheMode::Normal,
        provider_clock(2026, 10, 1),
        0,
    )
}

#[test]
fn a_read_only_cache_does_not_discard_a_successful_metar_fetch() {
    // A hand-built environment whose cache directory is `0555`: the observation can be read (no
    // entry yet, so the transport is asked), but the entry the provider writes afterwards cannot
    // be created. §3.1: the failed write must be swallowed like `Cache::read_or_fetch_json` does,
    // not propagated as a config error that throws the decoded observation away.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let cache_root = directory.path().join("cache");
    fs::create_dir_all(&cache_root).expect("the cache root is created");
    fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o555))
        .expect("the cache root becomes read-only");

    let paths = Paths {
        config_dir: directory.path().join("config"),
        config_file: directory.path().join("config/config.toml"),
        keys_file: directory.path().join("config/keys.toml"),
        cache_dir: cache_root.clone(),
        data_dir: directory.path().join("data"),
    };
    let transport = std::sync::Arc::new(StubTransport::new(vec![fixture_reply(
        "metar/ZBAA",
        "current.json",
    )]));
    let http = HttpClient::new(
        Box::new(std::sync::Arc::clone(&transport)),
        0,
        provider_clock(2026, 10, 1),
        0,
    );
    let cache = read_only_cache(&cache_root);
    let config = Config::default();
    let keys = KeyStore::new(&paths);
    let env = Env {
        http: &http,
        cache: &cache,
        config: &config,
        keys: &keys,
        quiet: true,
        verbose: 0,
    };

    let location: Location = cirrocast::provider::metar::placeholder_location("ZBAA");
    let report = Metar
        .fetch(
            &location,
            &FetchRequest::new(0, HourlyResolution::Hourly),
            &env,
        )
        .expect("a cache-write failure must not discard the observation");
    assert!(
        report.current.is_some(),
        "the decoded observation survives the unwritable cache"
    );

    // Keep the tempdir removal happy on platforms that refuse to unlink below a read-only
    // directory: the write failed, so the tree is empty and the mode can be restored.
    let _ = fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o755));
}

#[test]
fn a_variable_metar_wind_has_no_direction_rather_than_due_north() {
    // LFPG `010000Z VRB02KT`: the decoder already reports direction `None`; the provider used to
    // substitute `0`, which every renderer drew as a definite north wind.
    let run = ProviderRun::new(
        vec![fixture_reply("metar/LFPG", "current.json")],
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
    );
    let location = cirrocast::provider::metar::placeholder_location("LFPG");
    let report = run
        .fetch_with(&Metar, &location, 0)
        .expect("the LFPG observation decodes");
    let current = report.current.expect("the fixture has current conditions");
    assert_eq!(
        current.wind_dir_deg, None,
        "a VRB wind must not be reported as due north"
    );
    assert!(current.wind_kmh > 0.0, "the speed is still reported");
}
