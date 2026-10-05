// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The coverage-ranked `auto` expansion (step 24).
//!
//! `auto` is not a fixed list: it expands from the registry's `covers` metadata per resolved
//! location — an exact country match first, then a containing bounding box, then the global
//! keyless entries, each tier in registry order. These tests pin the expansion for a US, German,
//! Swedish, Norwegian, Hong-Kong and country-less point, prove that every keyless forecast backend
//! is reachable somewhere and that the station-only, archive-only and supplementary rows never
//! enter, and check that `-v` prints what was chosen.
//!
//! No test here opens a socket: the chains come from the registry, and the CLI case runs under the
//! sandbox's network guard, which refuses the request *after* the chain line was printed.

mod common;

use std::collections::BTreeSet;

use cirrocast::model::{Location, LocationSource};
use cirrocast::provider::{ProviderId, auto_chain, select_for};

/// A point with the given country code, at the given coordinates.
fn point(name: &str, country: Option<&str>, lat: f64, lon: f64) -> Location {
    Location {
        name: name.to_owned(),
        admin1: None,
        country: String::new(),
        country_code: country.map(str::to_owned),
        lat,
        lon,
        tz: chrono_tz::Tz::UTC,
        elevation_m: None,
        population: None,
        source: LocationSource::Geocoder,
        station: None,
    }
}

/// The probe points of the coverage table, with the chain each must produce.
fn probes() -> Vec<(Location, Vec<ProviderId>)> {
    vec![
        // A US point: the national service first, then the global keyless entries.
        (
            point("Norman", Some("US"), 35.22, -97.44),
            vec![ProviderId::Nws, ProviderId::OpenMeteo, ProviderId::MetNo],
        ),
        // A German point: Bright Sky covers Germany.
        (
            point("Berlin", Some("DE"), 52.52, 13.41),
            vec![
                ProviderId::BrightSky,
                ProviderId::OpenMeteo,
                ProviderId::MetNo,
            ],
        ),
        // A Swedish point: SMHI's bounding box covers the Nordics.
        (
            point("Stockholm", Some("SE"), 59.33, 18.07),
            vec![ProviderId::Smhi, ProviderId::OpenMeteo, ProviderId::MetNo],
        ),
        // Norway: MET Norway is its national service (tier 0) even though the same row answers
        // globally, and SMHI's box reaches southern Norway (tier 1) behind it.
        (
            point("Oslo", Some("NO"), 59.91, 10.75),
            vec![ProviderId::MetNo, ProviderId::Smhi, ProviderId::OpenMeteo],
        ),
        // Hong Kong: no keyless national backend (HKO contributes alerts only).
        (
            point("Hong Kong", Some("HK"), 22.32, 114.17),
            vec![ProviderId::OpenMeteo, ProviderId::MetNo],
        ),
        // No country code at all: the global tier alone, exactly as the pre-resolution expansion.
        (
            point("mid-Pacific", None, 0.0, -140.0),
            vec![ProviderId::OpenMeteo, ProviderId::MetNo],
        ),
    ]
}

#[test]
fn the_expansion_matches_the_coverage_table() {
    for (location, expected) in probes() {
        assert_eq!(
            auto_chain(Some(&location)),
            expected,
            "{}, {:?}",
            location.name,
            location.country_code
        );
        assert_eq!(
            select_for("auto", Some(&location)).expect("auto expands"),
            expected,
            "{} through the chain selector",
            location.name
        );
    }
    // Before a location is known the expansion is the global tier, which is what the CLI's
    // pre-flight sees.
    assert_eq!(
        auto_chain(None),
        vec![ProviderId::OpenMeteo, ProviderId::MetNo]
    );
}

#[test]
fn every_keyless_forecast_backend_is_reachable_and_the_rest_never_enter() {
    let mut reachable: BTreeSet<ProviderId> = BTreeSet::new();
    for (location, _) in probes() {
        let chain = auto_chain(Some(&location));
        let unique: BTreeSet<ProviderId> = chain.iter().copied().collect();
        assert_eq!(
            unique.len(),
            chain.len(),
            "{}: a chain has no duplicate entries: {chain:?}",
            location.name
        );
        reachable.extend(chain);
    }

    // Every registry row that could answer a forecast for a resolved place, keyless, must appear in
    // at least one expansion — that is what makes "auto" coverage-ranked rather than a hardcoded
    // shortlist.
    let expected: BTreeSet<ProviderId> = ProviderId::all()
        .into_iter()
        .filter(|id| {
            let meta = id.metadata();
            meta.implemented
                && !meta.requires_key
                && meta.location_kinds.city
                && !meta.marine
                && meta.max_days > 0
        })
        .collect();
    assert_eq!(reachable, expected);

    // The excluded shapes, spelled out: a station has to be asked for, an archive answers
    // `--date`/`--history` and a supplementary source is `--marine`.
    for id in [
        ProviderId::Metar,
        ProviderId::OpenMeteoArchive,
        ProviderId::OpenMeteoMarine,
    ] {
        assert!(
            !reachable.contains(&id),
            "{id} must never be part of `auto`"
        );
    }
    // …and every chain that does answer is keyless: `auto` never needs a credential.
    for id in &reachable {
        assert!(!id.metadata().requires_key, "{id} needs a key");
    }
}

#[test]
fn the_chain_is_ranked_by_tier_then_registry_order() {
    // Two entries in one tier keep registry order; a closer tier wins over a farther one even when
    // its registry position is later.
    let berlin = point("Berlin", Some("DE"), 52.52, 13.41);
    let chain = auto_chain(Some(&berlin));
    let positions: Vec<usize> = chain
        .iter()
        .map(|id| {
            ProviderId::all()
                .iter()
                .position(|known| known == id)
                .unwrap_or(0)
        })
        .collect();
    let global_start = chain
        .iter()
        .position(|id| id.metadata().covers == cirrocast::provider::Coverage::GLOBAL)
        .expect("the global tier is always present");
    assert!(
        positions[global_start..]
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "the global tier keeps registry order: {chain:?}"
    );
    assert_eq!(chain[0], ProviderId::BrightSky);
}

#[test]
fn the_verbose_line_names_the_location_and_the_chain() {
    // The CLI prints the expansion under `-v` before the fetch; the sandbox's network guard makes
    // the fetch itself fail, which is fine — the line is what this test reads.
    let sandbox = common::Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["-v", "-p", "auto", "-f", "plain", "Berlin"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("provider: auto for Berlin (52.52, 13.41, DE)"),
        "the expansion is not printed: {stderr}"
    );
    assert!(
        stderr.contains("brightsky") && stderr.contains("open-meteo"),
        "the chain names its entries: {stderr}"
    );
}
