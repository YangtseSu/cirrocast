// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Climate normals: the monthly comparison the normals panel renders.
//!
//! A climate normal is not a forecast reading: it is the mean of one calendar month's summary over
//! a fixed reference period (the WMO 1991–2020 window by default), computed here from NOAA NCEI's
//! Global Summary of the Month for the station nearest the location. The type lives in the model,
//! next to everything else a [`Report`](super::Report) carries, because the renderers may read the
//! model but never the module that fetches the data (`src/normals` owns the fetching), and
//! [`Report::normals`](super::Report) defaults to `None` so a document written before the block
//! existed still parses.
//!
//! The rules that shape the type:
//!
//! * **Canonical metric.** The three temperatures are °C and the precipitation a millimetre total
//!   for the calendar month — the same single conversion point every other reading goes through, so
//!   `--units` never reaches the wire and the cache stays unit-free.
//! * **The station and its distance travel with the reading.** GSOM stations are sparse, and a
//!   normal from 50 km away is still a normal, but presenting it as "your" climate without a word
//!   would be wrong: the panel names the station, its identifier and how far it sits from the
//!   requested point.
//! * **`years` is part of the reading.** A "normal" over eight years is not the WMO normal, and the
//!   number printed beside the values is what lets a reader discount a thin record; the fetcher
//!   refuses a month with fewer than 20 contributing years outright.

use serde::{Deserialize, Serialize};

/// One month's climate normal for the station nearest a location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normals {
    /// The station's own identifier, e.g. `USW00014837` (the search response's `stations[].id`).
    pub station: String,
    /// The station's name as the search response spells it, e.g. `MADISON DANE CO REGIONAL AIRPORT,
    /// WI US`.
    pub station_name: String,
    /// Great-circle distance from the requested point to the station, in kilometres.
    pub distance_km: f64,
    /// The reference period the values are averaged over, as configured, e.g. `1991-2020`.
    pub period: String,
    /// The calendar month the values are for, `1`–`12`.
    pub month: u8,
    /// Mean of the month's mean daily temperatures, °C.
    pub temp_mean_c: f32,
    /// Mean of the month's mean daily maxima, °C.
    pub temp_max_c: f32,
    /// Mean of the month's mean daily minima, °C.
    pub temp_min_c: f32,
    /// Mean of the month's precipitation totals, mm.
    pub precip_mm: f32,
    /// How many years of the period contributed; the fetcher requires at least 20.
    pub years: u16,
}
