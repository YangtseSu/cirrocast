// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The on-disk cache: TTL boundaries, modes, self-healing, atomic writes and the counters.
//!
//! Time comes from `FakeClock`, so the 599/600-second TTL boundary is an assertion rather than a
//! sleep, and the concurrency case is a real reader/writer pair on one directory.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use cirrocast::cache::{Cache, CacheKey, CacheMode, FakeClock};
use cirrocast::error::Error;

/// A fixed start instant, so the envelope timestamps are deterministic.
fn start() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// The key most cases use.
fn key() -> CacheKey {
    CacheKey::hash("geocode", "open-meteo|beijing|10|en")
}

/// A cache over a fresh temporary directory.
fn cache(mode: CacheMode, clock: &Arc<FakeClock>) -> (Cache, tempfile::TempDir) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let cache = reopen(directory.path(), mode, clock);
    (cache, directory)
}

/// Another cache view over the same root, sharing the clock.
fn reopen(root: &std::path::Path, mode: CacheMode, clock: &Arc<FakeClock>) -> Cache {
    let clock: Arc<dyn cirrocast::cache::Clock> = clock.clone();
    Cache::with_root(root, mode, clock, 0)
}

fn clock() -> Arc<FakeClock> {
    Arc::new(FakeClock::new(start()))
}

#[test]
fn entries_round_trip_and_expire_exactly_at_the_ttl() {
    let clock = clock();
    let (cache, _directory) = cache(CacheMode::Normal, &clock);
    cache
        .write(&key(), 200, "{\"results\":[]}", Duration::from_secs(600))
        .expect("the write succeeds");

    let entry = cache
        .read(&key())
        .expect("the entry is readable")
        .expect("the entry is fresh");
    assert_eq!(entry.status, 200);
    assert_eq!(entry.body, "{\"results\":[]}");
    assert_eq!(entry.ttl_secs, 600);
    assert_eq!(entry.key, "open-meteo|beijing|10|en");
    assert_eq!(
        entry.cache_schema_version,
        cirrocast::cache::CACHE_SCHEMA_VERSION
    );
    let envelope =
        std::fs::read_to_string(cache.entry_path(&key())).expect("the entry is readable");
    assert!(
        envelope.contains("\"fetched_at\": \"2023-11-14T22:13:20Z\""),
        "{envelope}"
    );

    clock.advance(Duration::from_secs(599));
    assert!(cache.read(&key()).expect("a read succeeds").is_some());
    clock.advance(Duration::from_secs(1));
    assert!(cache.read(&key()).expect("a read succeeds").is_none());
}

#[test]
fn a_future_envelope_version_is_a_miss_and_a_corrupt_entry_self_heals() {
    let clock = clock();
    let (cache, _directory) = cache(CacheMode::Normal, &clock);
    cache
        .write(&key(), 200, "{\"results\":[]}", Duration::from_secs(600))
        .expect("the write succeeds");

    let path = cache.entry_path(&key());
    let text = std::fs::read_to_string(&path).expect("the entry is readable");
    std::fs::write(
        &path,
        text.replace(
            "\"cache_schema_version\": 1",
            "\"cache_schema_version\": 99",
        ),
    )
    .expect("the entry is writable");
    assert!(cache.read(&key()).expect("a read succeeds").is_none());

    std::fs::write(&path, "{ not json").expect("the entry is writable");
    assert!(cache.read(&key()).expect("a read succeeds").is_none());

    let fetches = AtomicUsize::new(0);
    let value: serde_json::Value = cache
        .read_or_fetch_json(
            &key(),
            Duration::from_secs(600),
            "test",
            "the test key",
            || {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok((200, "{\"results\":[\"recovered\"]}".to_owned()))
            },
        )
        .expect("the refetch succeeds");
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert_eq!(value["results"][0], "recovered");
    assert!(cache.read(&key()).expect("a read succeeds").is_some());

    let again: serde_json::Value = cache
        .read_or_fetch_json(
            &key(),
            Duration::from_secs(600),
            "test",
            "the test key",
            || {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok((200, "{}".to_owned()))
            },
        )
        .expect("the repaired entry parses");
    assert_eq!(again["results"][0], "recovered");
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
}

#[test]
fn no_cache_reads_and_writes_nothing() {
    let clock = clock();
    let (cache, directory) = cache(CacheMode::NoCache, &clock);
    cache
        .write(&key(), 200, "{}", Duration::from_secs(600))
        .expect("a no-op write still succeeds");
    assert!(cache.read(&key()).expect("a read succeeds").is_none());
    assert!(
        !directory.path().join("geocode").exists(),
        "no-cache must not create anything"
    );

    let fetches = AtomicUsize::new(0);
    cache
        .read_or_fetch_json::<serde_json::Value>(
            &key(),
            Duration::from_secs(600),
            "test",
            "the test key",
            || {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok((200, "{}".to_owned()))
            },
        )
        .expect("the fetch result is parsed");
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert!(!directory.path().join("geocode").exists());
}

#[test]
fn refresh_bypasses_a_fresh_entry_and_replaces_it() {
    let clock = clock();
    let (cache, _directory) = cache(CacheMode::Normal, &clock);
    cache
        .write(&key(), 200, "{\"generation\":1}", Duration::from_secs(600))
        .expect("the write succeeds");

    let refreshing = reopen(cache.root(), CacheMode::Refresh, &clock);
    assert!(refreshing.read(&key()).expect("a read succeeds").is_none());
    let value: serde_json::Value = refreshing
        .read_or_fetch_json(
            &key(),
            Duration::from_secs(600),
            "test",
            "the test key",
            || Ok((200, "{\"generation\":2}".to_owned())),
        )
        .expect("the refreshed body parses");
    assert_eq!(value["generation"], 2);
    assert_eq!(
        cache
            .read(&key())
            .expect("a read succeeds")
            .expect("the entry is fresh")
            .body,
        "{\"generation\":2}"
    );
}

#[test]
fn offline_serves_hits_and_fails_loudly_on_misses() {
    let clock = clock();
    let (cache, _directory) = cache(CacheMode::Normal, &clock);
    cache
        .write(&key(), 200, "{\"cached\":true}", Duration::from_secs(600))
        .expect("the write succeeds");

    let offline = reopen(cache.root(), CacheMode::Offline, &clock);
    let fetches = AtomicUsize::new(0);
    let value: serde_json::Value = offline
        .read_or_fetch_json(
            &key(),
            Duration::from_secs(600),
            "test",
            "the test key",
            || {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok((200, "{}".to_owned()))
            },
        )
        .expect("the cached body is served");
    assert_eq!(value["cached"], true);
    assert_eq!(fetches.load(Ordering::SeqCst), 0);

    let missing = CacheKey::hash("geocode", "open-meteo|shanghai|10|en");
    let error = offline
        .read_or_fetch_json::<serde_json::Value>(
            &missing,
            Duration::from_secs(600),
            "test",
            "the test key",
            || panic!("offline mode must not fetch"),
        )
        .expect_err("a miss is a hard failure");
    assert_eq!(error.exit_code(), 3);
    assert!(
        error
            .to_string()
            .contains("offline mode: no cached test answer for the test key at geocode/"),
        "{error}"
    );
    assert!(
        error.to_string().contains("rerun without `--offline`"),
        "{error}"
    );
    assert!(error.to_string().contains(&format!(
        "{}.json",
        missing.path().file_stem().expect("a file stem").to_string_lossy()
    )));

    offline
        .write(&key(), 200, "{}", Duration::from_secs(600))
        .expect("an offline write is a no-op");
    assert!(offline.clean(true).is_err(), "offline refuses deletions");
    match offline.clean(true) {
        Err(Error::Usage(message)) => assert!(message.contains("cache writes are disabled")),
        other => panic!("expected a usage error, got {other:?}"),
    }
}

#[test]
fn concurrent_readers_never_see_a_half_written_entry() {
    let clock = clock();
    let (cache, directory) = cache(CacheMode::Normal, &clock);
    let cache = Arc::new(cache);
    let bodies: Vec<String> = (0_usize..40)
        .map(|index| {
            format!(
                "{{\"generation\":{index},\"padding\":\"{}\"}}",
                "x".repeat(index * 37)
            )
        })
        .collect();
    let written: Vec<String> = bodies.clone();

    let writer_cache = Arc::clone(&cache);
    let writer = std::thread::spawn(move || {
        for body in &written {
            writer_cache
                .write(&key(), 200, body, Duration::from_secs(600))
                .expect("the write succeeds");
        }
    });

    let mut observed = 0_usize;
    while !writer.is_finished() {
        if let Some(entry) = cache.read(&key()).expect("a read succeeds") {
            assert!(
                bodies.contains(&entry.body),
                "a reader observed a partial body: {} bytes",
                entry.body.len()
            );
            observed += 1;
        }
    }
    writer
        .join()
        .expect("the writer finishes without panicking");
    assert!(observed > 0, "the reader never saw a complete entry");
    for entry in std::fs::read_dir(directory.path().join("geocode")).expect("the directory exists")
    {
        let name = entry.expect("a readable entry").file_name();
        assert!(
            !name.to_string_lossy().contains(".tmp."),
            "a temporary file survived: {}",
            name.to_string_lossy()
        );
    }
}

#[test]
fn stat_and_clean_count_what_they_say() {
    let clock = clock();
    let (cache, _directory) = cache(CacheMode::Normal, &clock);
    let fresh = CacheKey::hash("geocode", "open-meteo|beijing|10|en");
    let stale = CacheKey::hash("geocode", "open-meteo|old|10|en");
    let ip = CacheKey::ip("ipwho-is");
    cache
        .write(&fresh, 200, "{\"a\":1}", Duration::from_secs(600))
        .expect("the write succeeds");
    cache
        .write(&stale, 200, "{\"b\":2}", Duration::from_secs(60))
        .expect("the write succeeds");
    cache
        .write(&ip, 200, "{\"c\":3}", Duration::from_hours(24))
        .expect("the write succeeds");
    clock.advance(Duration::from_secs(120));

    let stat = cache.stat().expect("stat succeeds");
    assert_eq!(
        stat.namespaces
            .iter()
            .map(|namespace| namespace.name)
            .collect::<Vec<_>>(),
        ["weather", "geocode", "ip", "station"]
    );
    assert_eq!(stat.namespaces[0].entries, 0);
    assert_eq!(stat.namespaces[0].bytes, 0);
    assert_eq!(stat.namespaces[1].entries, 2);
    assert!(stat.namespaces[1].bytes > 0);
    assert_eq!(stat.namespaces[2].entries, 1);
    // The station namespace is empty in this fixture, and `cache clean` must not touch it.
    assert_eq!(stat.namespaces[3].entries, 0);
    assert!(stat.namespaces[1].oldest.is_some());
    assert_eq!(stat.namespaces[1].newest, stat.namespaces[1].oldest);

    assert_eq!(cache.clean(false).expect("clean succeeds").removed, 1);
    assert!(cache.read(&stale).expect("a read succeeds").is_none());
    assert!(cache.read(&fresh).expect("a read succeeds").is_some());
    assert_eq!(cache.clean(true).expect("clean succeeds").removed, 2);
    assert!(cache.read(&fresh).expect("a read succeeds").is_none());
}

#[test]
fn state_files_live_outside_the_entry_namespaces() {
    let clock = clock();
    let (cache, _directory) = cache(CacheMode::Normal, &clock);
    assert!(
        cache
            .read_state("ratelimit/nominatim.json")
            .expect("a read succeeds")
            .is_none()
    );
    cache
        .write_state("ratelimit/nominatim.json", "{\"last_request_unix_ms\":1}")
        .expect("the state write succeeds");
    assert_eq!(
        cache
            .read_state("ratelimit/nominatim.json")
            .expect("a read succeeds")
            .as_deref(),
        Some("{\"last_request_unix_ms\":1}")
    );
    assert!(
        cache
            .read(&CacheKey::hash("ratelimit", "x"))
            .expect("a read succeeds")
            .is_none()
    );
}
