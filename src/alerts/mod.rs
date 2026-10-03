// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Severe-weather alerts: their own source registry, independent of the weather chain.
//!
//! The alert sources answer different questions than the forecast backends: they cover different
//! jurisdictions (`NWS`: the US; `HKO`: Hong Kong; `MeteoAlarm`: the EUMETNET members; `QWeather`: China)
//! or aggregate the whole world (WMO SWIC, FPAS), and which of them applies is a property of the
//! *resolved location*, not of the provider chain. A source is therefore selected by coverage
//! ([`selection`]), fetched through the shared HTTP client and cache, normalised to the CAP-shaped
//! [`Alert`](crate::model::Alert) model, and only then filtered, de-duplicated and sorted.
//!
//! The data types live in `src/model/alert.rs` and are re-exported here: `model::Report` carries
//! the alerts, and the render layer may read the model but not this module.
//!
//! Each adapter keeps its own parsing:
//!
//! * [`nws`] reads the `GeoJSON` flavour of CAP;
//! * [`meteoalarm`], [`wmoswic`] and [`fpas`] read CAP 1.2 through the shared [`cap`] reader;
//! * [`qweather`] reads its JSON warning array;
//! * [`hko`] reads the Observatory's warning summary and detail documents.

pub mod cap;
pub mod geometry;

pub use crate::model::{Alert, AlertSource, Certainty, Severity, Urgency};
