// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open-Meteo's historical archive (ERA5 / ERA5-Land / IFS reanalysis) as a backend.
//!
//! The archive is the same service family as [`super::open_meteo`], on a different host and with a
//! different time direction: it answers for dates that have already happened, from **1940-01-01**
//! onward, and has no forecast horizon at all (`max_days: 0` in the registry row, whose
//! `history_days` is what the CLI's `--date`/`--history` validation reads).
//!
//! One request per fetch, over an explicit `start_date`/`end_date` span — the window is mandatory
//! here, because "the next N days" is not a question the reanalysis can answer. Two consequences
//! the payload itself does not spell out:
//!
//! 1. **There is no `current` block.** The reanalysis is not an observation stream and the archive
//!    endpoint answers no `current` object at all, so the report's current conditions are `None`
//!    and every renderer's "now" panel takes the empty shape it already handles.
//! 2. **ERA5 lags the present by about five days** (the newest well-covered day is roughly
//!    `today - 5`). A window whose end falls inside that latency band is not in the archive yet, so
//!    it is served by the forecast API through [`open_meteo::fetch_window`] — the seam the two
//!    backends share. A window that is not entirely in the past is refused instead: the archive
//!    answers history only, and a window reaching today would be keyed exactly like a plain
//!    forecast (the forecast API keys on the location-local today), which is the cache collision
//!    this module must not create.
//!
//! The decode is the shared Open-Meteo one ([`open_meteo::report_for`]): the archive's payload is
//! the same shape minus `current`, so only the id, the URL and the cache key differ. The request
//! deliberately omits `precipitation_probability` — the reanalysis has no such variable and
//! upstream answers an all-`null` array for it — while `visibility` is still requested, because the
//! shared hourly decoder requires that array to be present; the nulls upstream answers for it
//! become `None` rather than a fabricated distance.
//!
//! Attribution: Open-Meteo's data is CC BY 4.0 (the reanalysis behind it is Copernicus/ERA5), and
//! the registry row carries the line the renderers print.

use std::time::Duration;

use chrono::{Datelike, Days};

use super::open_meteo::{self, DAILY_VARIABLES, ForecastResponse};
use super::{
    Capabilities, DateWindow, Env, FetchRequest, JsonFetch, Provider, ProviderId, fetch_json,
    local_today,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Location, Report};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "open-meteo-archive";

/// The archive endpoint.
pub const BASE: &str = "https://archive-api.open-meteo.com/v1/archive";

/// Hourly variables, in the fixed order the request uses: the forecast list without
/// `precipitation_probability`, which the reanalysis has no counterpart for (upstream answers an
/// all-`null` array of `undefined` unit for it). `visibility` stays in, because the shared hourly
/// decoder requires its array to be present; the nulls upstream answers become `None`.
const HOURLY_VARIABLES: &str = "temperature_2m,apparent_temperature,precipitation,weather_code,\
wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility";

/// The first year the reanalysis covers. The archive starts at `1940-01-01`, the first day of that
/// year, so a window starting in this year or later is inside the covered range.
const EARLIEST_YEAR: i32 = 1940;

/// How many days behind the present the reanalysis runs: a date after `today - LATENCY_DAYS` has
/// not been assimilated yet, so the forecast API serves it instead.
const LATENCY_DAYS: u64 = 5;

/// The Open-Meteo archive backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenMeteoArchive;

impl Provider for OpenMeteoArchive {
    fn id(&self) -> ProviderId {
        ProviderId::OpenMeteoArchive
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::OpenMeteoArchive.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let Some(window) = req.window else {
            return Err(Error::Usage(
                "open-meteo-archive is archive only; pass `--date <YYYY-MM-DD>` or `--history <N>d`"
                    .to_owned(),
            ));
        };
        if window.start.year() < EARLIEST_YEAR {
            return Err(Error::Usage(format!(
                "open-meteo-archive covers dates from 1940-01-01 onward; {} is before the reanalysis",
                window.start
            )));
        }

        let today = local_today(env, loc.tz);
        if window.end >= today {
            return Err(Error::Usage(format!(
                "open-meteo-archive is archive only; {} is not in the past (ask a forecast backend for it)",
                window.end
            )));
        }
        // The seam: the newest days are not in the reanalysis yet, so the forecast API — which
        // already has them — answers for the window. See the module documentation.
        if window.end > today - Days::new(LATENCY_DAYS) {
            return open_meteo::fetch_window(loc, window, env);
        }

        let days = window.days();
        let key = CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, window.end);
        let request = archive_request(loc, window);
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

        let response: ForecastResponse = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::OpenMeteoArchive,
                request: request.clone(),
                key,
                ttl,
                what: "reanalysis",
            },
        )?;

        open_meteo::report_for(
            ProviderId::OpenMeteoArchive,
            &response,
            loc,
            request.redacted_url(),
            days,
            env,
        )
    }
}

/// The archive request: an explicit span instead of a day count, and no `current` block (the
/// endpoint has none to give). The parameter order is fixed and the tests assert it verbatim.
fn archive_request(loc: &Location, window: DateWindow) -> HttpRequest {
    HttpRequest::get(BASE)
        .query("latitude", format!("{:.4}", loc.lat))
        .query("longitude", format!("{:.4}", loc.lon))
        .query("start_date", window.start.format("%Y-%m-%d").to_string())
        .query("end_date", window.end.format("%Y-%m-%d").to_string())
        .query("hourly", HOURLY_VARIABLES)
        .query("daily", DAILY_VARIABLES)
        .query("timezone", "auto")
        .query("temperature_unit", "celsius")
        .query("wind_speed_unit", "kmh")
        .query("precipitation_unit", "mm")
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, NaiveDate};

    use super::{BASE, EARLIEST_YEAR, LATENCY_DAYS, archive_request};
    use crate::model::{Location, LocationSource};
    use crate::provider::DateWindow;

    fn berlin() -> Location {
        Location {
            name: "Berlin".to_owned(),
            admin1: None,
            country: "Germany".to_owned(),
            country_code: Some("DE".to_owned()),
            lat: 52.52,
            lon: 13.405,
            tz: chrono_tz::Tz::Europe__Berlin,
            elevation_m: None,
            population: None,
            source: LocationSource::Geocoder,
            station: None,
        }
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
    }

    #[test]
    fn the_covered_range_starts_at_the_first_day_of_1940() {
        assert_eq!(EARLIEST_YEAR, 1940);
        // The check compares calendar years, which is exact because 1940-01-01 is the first day of
        // its year: every later date in 1940 is inside the covered range.
        assert!(date(1940, 1, 1).year() >= EARLIEST_YEAR);
        assert!(date(1939, 12, 31).year() < EARLIEST_YEAR);
        assert_eq!(LATENCY_DAYS, 5);
    }

    #[test]
    fn the_request_is_the_archive_path_with_the_span_and_no_current_block() {
        let request = archive_request(
            &berlin(),
            DateWindow {
                start: date(2026, 9, 14),
                end: date(2026, 9, 15),
            },
        );
        assert_eq!(request.url(), BASE);
        let pairs: Vec<(&str, &str)> = request
            .query_pairs()
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("latitude", "52.5200"),
                ("longitude", "13.4050"),
                ("start_date", "2026-09-14"),
                ("end_date", "2026-09-15"),
                (
                    "hourly",
                    "temperature_2m,apparent_temperature,precipitation,weather_code,\
wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility"
                ),
                (
                    "daily",
                    "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset"
                ),
                ("timezone", "auto"),
                ("temperature_unit", "celsius"),
                ("wind_speed_unit", "kmh"),
                ("precipitation_unit", "mm"),
            ]
        );
        assert!(
            !pairs.iter().any(|(name, _)| *name == "current"),
            "the archive endpoint has no current block to ask for"
        );
    }
}
