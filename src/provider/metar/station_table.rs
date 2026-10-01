// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The embedded station table: the no-I/O fast path for the stations people ask for most.
//!
//! Every METAR request needs the station's name, coordinates, elevation and time zone before it
//! can build a [`crate::model::Location`]. The upstream `stationinfo` endpoint serves all of that,
//! but only over the network: a lookup here answers the common identifiers without a request, and
//! the `stationinfo` endpoint extends the same answers to every other station in the world, one
//! cached request per station per thirty days.
//!
//! **Provenance.** `name`, `state`, `country`, `lat`, `lon` and `elev_m` are transcribed from the
//! `stationinfo` responses recorded under `tests/fixtures/stationinfo/` — NOAA/NWS station
//! metadata, which is US government public domain (see
//! `LICENSES/LicenseRef-US-Government-Public-Domain.txt`). The `tz` column is not upstream data:
//! it is [`crate::geo::tz::lookup`]'s answer for the row's coordinates, computed when the row was
//! added, because neither `stationinfo` nor a METAR report carries a time zone.
//!
//! **Ordering.** The array is sorted by ICAO and every query is a binary search, so the table can
//! grow without slowing a lookup down. A row is a *static claim*: the coordinates do not move, and
//! a station that closes keeps answering with its last known position, which is the same thing the
//! cached upstream row would do.

/// One station as the table records it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    /// ICAO identifier, upper case; the sort key and the lookup key.
    pub icao: &'static str,
    /// The place name as `stationinfo`'s `site` field reports it.
    pub name: &'static str,
    /// The first level administrative division, as `stationinfo`'s `state` field reports it.
    pub state: &'static str,
    /// ISO 3166-1 alpha 2 country code, as `stationinfo`'s `country` field reports it.
    pub country: &'static str,
    /// Latitude in degrees, WGS 84.
    pub lat: f64,
    /// Longitude in degrees, WGS 84.
    pub lon: f64,
    /// Elevation above sea level in metres, when the record has one.
    pub elev_m: Option<f64>,
    /// IANA time zone of the coordinates, from [`crate::geo::tz::lookup`].
    pub tz: &'static str,
}

/// The table, ICAO-sorted.
pub static STATIONS: &[Station] = &[
    Station {
        icao: "BIKF",
        name: "Keflavik Intl",
        state: "SP",
        country: "IS",
        lat: 63.987,
        lon: -22.614,
        elev_m: Some(49.0),
        tz: "Atlantic/Reykjavik",
    },
    Station {
        icao: "CYQB",
        name: "Quebec/Lesage Intl",
        state: "QC",
        country: "CA",
        lat: 46.791,
        lon: -71.396,
        elev_m: Some(73.0),
        tz: "America/Toronto",
    },
    Station {
        icao: "CYUL",
        name: "Montreal/Trudeau Intl",
        state: "QC",
        country: "CA",
        lat: 45.468,
        lon: -73.742,
        elev_m: Some(31.0),
        tz: "America/Toronto",
    },
    Station {
        icao: "CYVR",
        name: "Vancouver Intl",
        state: "BC",
        country: "CA",
        lat: 49.183,
        lon: -123.168,
        elev_m: Some(2.0),
        tz: "America/Vancouver",
    },
    Station {
        icao: "CYYC",
        name: "Calgary Intl",
        state: "AB",
        country: "CA",
        lat: 51.116,
        lon: -114.011,
        elev_m: Some(1085.0),
        tz: "America/Edmonton",
    },
    Station {
        icao: "CYYZ",
        name: "Toronto/Pearson Intl",
        state: "ON",
        country: "CA",
        lat: 43.679,
        lon: -79.629,
        elev_m: Some(171.0),
        tz: "America/Toronto",
    },
    Station {
        icao: "EDDF",
        name: "Frankfurt/Main Arpt",
        state: "HE",
        country: "DE",
        lat: 50.045,
        lon: 8.598,
        elev_m: Some(113.0),
        tz: "Europe/Berlin",
    },
    Station {
        icao: "EDDM",
        name: "Munich Intl",
        state: "BY",
        country: "DE",
        lat: 48.348,
        lon: 11.813,
        elev_m: Some(445.0),
        tz: "Europe/Berlin",
    },
    Station {
        icao: "EFHK",
        name: "Helsinki/Vantaa Arpt",
        state: "UU",
        country: "FI",
        lat: 60.327,
        lon: 24.957,
        elev_m: Some(56.0),
        tz: "Europe/Helsinki",
    },
    Station {
        icao: "EGLL",
        name: "London/Heathrow Intl",
        state: "EN",
        country: "GB",
        lat: 51.477,
        lon: -0.461,
        elev_m: Some(26.0),
        tz: "Europe/London",
    },
    Station {
        icao: "EHAM",
        name: "Amsterdam/Schiphol Arpt",
        state: "NH",
        country: "NL",
        lat: 52.315,
        lon: 4.79,
        elev_m: Some(-2.0),
        tz: "Europe/Amsterdam",
    },
    Station {
        icao: "EIDW",
        name: "Dublin Arpt",
        state: "L",
        country: "IE",
        lat: 53.422,
        lon: -6.298,
        elev_m: Some(75.0),
        tz: "Europe/Dublin",
    },
    Station {
        icao: "EKCH",
        name: "Kobenhavn Intl",
        state: "HO",
        country: "DK",
        lat: 55.618,
        lon: 12.656,
        elev_m: Some(4.0),
        tz: "Europe/Copenhagen",
    },
    Station {
        icao: "ENGM",
        name: "Oslo/Gardermoen Arpt",
        state: "AK",
        country: "NO",
        lat: 60.201,
        lon: 11.08,
        elev_m: Some(204.0),
        tz: "Europe/Oslo",
    },
    Station {
        icao: "EPWA",
        name: "Warsaw/Chopin Arpt",
        state: "MZ",
        country: "PL",
        lat: 52.163,
        lon: 20.961,
        elev_m: Some(107.0),
        tz: "Europe/Warsaw",
    },
    Station {
        icao: "ESSA",
        name: "Stockholm/Arlanda Arpt",
        state: "AB",
        country: "SE",
        lat: 59.664,
        lon: 17.92,
        elev_m: Some(36.0),
        tz: "Europe/Stockholm",
    },
    Station {
        icao: "FZAA",
        name: "Kinshasa/NDjili Arpt",
        state: "KN",
        country: "CD",
        lat: -4.387,
        lon: 15.448,
        elev_m: Some(309.0),
        tz: "Africa/Kinshasa",
    },
    Station {
        icao: "KATL",
        name: "Atlanta/Hartsfield-Jackson Intl",
        state: "GA",
        country: "US",
        lat: 33.62972,
        lon: -84.44223,
        elev_m: Some(309.0),
        tz: "America/New_York",
    },
    Station {
        icao: "KBOS",
        name: "Boston/Logan Intl",
        state: "MA",
        country: "US",
        lat: 42.36057,
        lon: -71.00974,
        elev_m: Some(4.0),
        tz: "America/New_York",
    },
    Station {
        icao: "KDEN",
        name: "Denver Intl",
        state: "CO",
        country: "US",
        lat: 39.84657,
        lon: -104.65623,
        elev_m: Some(1656.0),
        tz: "America/Denver",
    },
    Station {
        icao: "KDFW",
        name: "Dallas-Ft Worth Intl",
        state: "TX",
        country: "US",
        lat: 32.89744,
        lon: -97.02195,
        elev_m: Some(168.0),
        tz: "America/Chicago",
    },
    Station {
        icao: "KDTW",
        name: "Detroit/Metro Wayne Cnty",
        state: "MI",
        country: "US",
        lat: 42.23112,
        lon: -83.33122,
        elev_m: Some(192.0),
        tz: "America/Detroit",
    },
    Station {
        icao: "KJFK",
        name: "New York/JF Kennedy Intl",
        state: "NY",
        country: "US",
        lat: 40.63916,
        lon: -73.76394,
        elev_m: Some(3.0),
        tz: "America/New_York",
    },
    Station {
        icao: "KLAS",
        name: "Las Vegas/Reid Intl",
        state: "NV",
        country: "US",
        lat: 36.07188,
        lon: -115.16343,
        elev_m: Some(662.0),
        tz: "America/Los_Angeles",
    },
    Station {
        icao: "KLAX",
        name: "Los Angeles Intl",
        state: "CA",
        country: "US",
        lat: 33.93817,
        lon: -118.3866,
        elev_m: Some(30.0),
        tz: "America/Los_Angeles",
    },
    Station {
        icao: "KMIA",
        name: "Miami Intl",
        state: "FL",
        country: "US",
        lat: 25.78806,
        lon: -80.31692,
        elev_m: Some(1.0),
        tz: "America/New_York",
    },
    Station {
        icao: "KMSP",
        name: "Minneapolis-St Paul Intl",
        state: "MN",
        country: "US",
        lat: 44.88523,
        lon: -93.23132,
        elev_m: Some(255.0),
        tz: "America/Chicago",
    },
    Station {
        icao: "KORD",
        name: "Chicago/O'Hare Intl",
        state: "IL",
        country: "US",
        lat: 41.96017,
        lon: -87.93161,
        elev_m: Some(202.0),
        tz: "America/Chicago",
    },
    Station {
        icao: "KPDX",
        name: "Portland Intl",
        state: "OR",
        country: "US",
        lat: 45.59578,
        lon: -122.60917,
        elev_m: Some(7.0),
        tz: "America/Los_Angeles",
    },
    Station {
        icao: "KPHX",
        name: "Phoenix/Sky Harbor Intl",
        state: "AZ",
        country: "US",
        lat: 33.42779,
        lon: -112.00366,
        elev_m: Some(338.0),
        tz: "America/Phoenix",
    },
    Station {
        icao: "KSEA",
        name: "Seattle-Tacoma Intl",
        state: "WA",
        country: "US",
        lat: 47.44467,
        lon: -122.31442,
        elev_m: Some(115.0),
        tz: "America/Los_Angeles",
    },
    Station {
        icao: "KSFO",
        name: "San Francisco Intl",
        state: "CA",
        country: "US",
        lat: 37.61961,
        lon: -122.36561,
        elev_m: Some(2.0),
        tz: "America/Los_Angeles",
    },
    Station {
        icao: "KSLC",
        name: "Salt Lake City Intl",
        state: "UT",
        country: "US",
        lat: 40.7707,
        lon: -111.96503,
        elev_m: Some(1286.0),
        tz: "America/Denver",
    },
    Station {
        icao: "KSMF",
        name: "Sacramento Intl",
        state: "CA",
        country: "US",
        lat: 38.70069,
        lon: -121.59479,
        elev_m: Some(7.0),
        tz: "America/Los_Angeles",
    },
    Station {
        icao: "LEBL",
        name: "Barcelona/Prat Arpt",
        state: "CT",
        country: "ES",
        lat: 41.293,
        lon: 2.07,
        elev_m: Some(2.0),
        tz: "Europe/Madrid",
    },
    Station {
        icao: "LEMD",
        name: "Madrid/Barajas Arpt",
        state: "M",
        country: "ES",
        lat: 40.466,
        lon: -3.555,
        elev_m: Some(589.0),
        tz: "Europe/Madrid",
    },
    Station {
        icao: "LFPG",
        name: "Paris/De Gaulle Arpt",
        state: "ID",
        country: "FR",
        lat: 49.015,
        lon: 2.534,
        elev_m: Some(107.0),
        tz: "Europe/Paris",
    },
    Station {
        icao: "LHBP",
        name: "Budapest/Liszt Intl",
        state: "BU",
        country: "HU",
        lat: 47.437,
        lon: 19.256,
        elev_m: Some(141.0),
        tz: "Europe/Budapest",
    },
    Station {
        icao: "LIRF",
        name: "Rome/Fiumicino",
        state: "RM",
        country: "IT",
        lat: 41.8,
        lon: 12.239,
        elev_m: Some(2.0),
        tz: "Europe/Rome",
    },
    Station {
        icao: "LOWW",
        name: "Vienna Intl",
        state: "NE",
        country: "AT",
        lat: 48.117,
        lon: 16.582,
        elev_m: Some(179.0),
        tz: "Europe/Vienna",
    },
    Station {
        icao: "LPPT",
        name: "Lisbon Arpt",
        state: "LI",
        country: "PT",
        lat: 38.781,
        lon: -9.136,
        elev_m: Some(98.0),
        tz: "Europe/Lisbon",
    },
    Station {
        icao: "LSZH",
        name: "Zürich Intl Arpt",
        state: "ZH",
        country: "CH",
        lat: 47.48,
        lon: 8.536,
        elev_m: Some(424.0),
        tz: "Europe/Zurich",
    },
    Station {
        icao: "OMDB",
        name: "Dubai Intl",
        state: "DU",
        country: "AE",
        lat: 25.254,
        lon: 55.366,
        elev_m: Some(5.0),
        tz: "Asia/Dubai",
    },
    Station {
        icao: "PASC",
        name: "Deadhorse Arpt",
        state: "AK",
        country: "US",
        lat: 70.191,
        lon: -148.48,
        elev_m: Some(17.0),
        tz: "America/Anchorage",
    },
    Station {
        icao: "PHNL",
        name: "Honolulu Intl",
        state: "HI",
        country: "US",
        lat: 21.31505,
        lon: -157.924,
        elev_m: Some(2.0),
        tz: "Pacific/Honolulu",
    },
    Station {
        icao: "RJAA",
        name: "Narita Intl",
        state: "12",
        country: "JP",
        lat: 35.765,
        lon: 140.386,
        elev_m: Some(36.0),
        tz: "Asia/Tokyo",
    },
    Station {
        icao: "RJTT",
        name: "Tokyo/Haneda Intl",
        state: "13",
        country: "JP",
        lat: 35.553,
        lon: 139.781,
        elev_m: Some(5.0),
        tz: "Asia/Tokyo",
    },
    Station {
        icao: "RKSI",
        name: "Seoul/Incheon Intl",
        state: "28",
        country: "KR",
        lat: 37.469,
        lon: 126.451,
        elev_m: Some(7.0),
        tz: "Asia/Seoul",
    },
    Station {
        icao: "UUEE",
        name: "Moscow/Sheremetyevo Intl",
        state: "MO",
        country: "RU",
        lat: 55.973,
        lon: 37.415,
        elev_m: Some(186.0),
        tz: "Europe/Moscow",
    },
    Station {
        icao: "VHHH",
        name: "Hong Kong Intl",
        state: "HK",
        country: "HK",
        lat: 22.309,
        lon: 113.922,
        elev_m: Some(9.0),
        tz: "Asia/Hong_Kong",
    },
    Station {
        icao: "WSSS",
        name: "Singapore/Changi Intl",
        state: "4",
        country: "SG",
        lat: 1.368,
        lon: 103.982,
        elev_m: Some(17.0),
        tz: "Asia/Singapore",
    },
    Station {
        icao: "YPDN",
        name: "Darwin Intl",
        state: "NT",
        country: "AU",
        lat: -12.424,
        lon: 130.893,
        elev_m: Some(32.0),
        tz: "Australia/Darwin",
    },
    Station {
        icao: "ZBAA",
        name: "Beijing Intl",
        state: "BJ",
        country: "CN",
        lat: 40.082,
        lon: 116.603,
        elev_m: Some(31.0),
        tz: "Asia/Shanghai",
    },
    Station {
        icao: "ZGGG",
        name: "Guangzhou/Baiyun Intl",
        state: "GD",
        country: "CN",
        lat: 23.392,
        lon: 113.307,
        elev_m: Some(11.0),
        tz: "Asia/Shanghai",
    },
    Station {
        icao: "ZSPD",
        name: "Shanghai/Pudong Intl",
        state: "SH",
        country: "CN",
        lat: 31.146,
        lon: 121.8,
        elev_m: Some(4.0),
        tz: "Asia/Shanghai",
    },
];

/// The station behind an ICAO identifier.
///
/// The lookup is case-insensitive, so a hand-typed `kjfk` finds the row; the CLI normalises the
/// identifier before it gets this far anyway.
#[must_use]
pub fn lookup(icao: &str) -> Option<&'static Station> {
    let icao = icao.trim().to_ascii_uppercase();
    STATIONS
        .binary_search_by(|station| station.icao.cmp(icao.as_str()))
        .ok()
        .and_then(|index| STATIONS.get(index))
}

/// The table row nearest to a coordinate pair, by great-circle distance.
///
/// This is how a `@lat,lon` location reaches an observation: METAR is served per station and the
/// table is the only station index in the binary. The caller reports the chosen station and its
/// distance under `-v`, because "nearest of fifty" is an approximation the user has to be able to
/// see — the observation is the airport's, not the exact point's.
#[must_use]
pub fn nearest(lat: f64, lon: f64) -> Option<&'static Station> {
    nearest_with_distance(lat, lon).map(|(station, _distance)| station)
}

/// [`nearest`] with the great-circle distance in kilometres, for the `-v` line.
#[must_use]
pub fn nearest_with_distance(lat: f64, lon: f64) -> Option<(&'static Station, f64)> {
    STATIONS
        .iter()
        .map(|station| (station, distance_km(station.lat, station.lon, lat, lon)))
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
}

/// Great-circle distance in kilometres (haversine), used only to rank table rows.
#[must_use]
pub fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0088;
    let to_radians = |degrees: f64| degrees.to_radians();
    let (phi1, phi2) = (to_radians(lat1), to_radians(lat2));
    let delta_phi = phi2 - phi1;
    let delta_lambda = to_radians(lon2 - lon1);
    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::{STATIONS, lookup, nearest, nearest_with_distance};

    #[test]
    fn the_table_is_sorted_by_icao_and_free_of_duplicates() {
        for pair in STATIONS.windows(2) {
            let (left, right) = (&pair[0], &pair[1]);
            assert!(
                left.icao < right.icao,
                "{} must sort before {}",
                left.icao,
                right.icao
            );
        }
    }

    #[test]
    fn every_zone_in_the_table_parses() {
        for station in STATIONS {
            let zone = station
                .tz
                .parse::<chrono_tz::Tz>()
                .unwrap_or_else(|_| panic!("{}: no zone `{}`", station.icao, station.tz));
            assert_eq!(zone.name(), station.tz);
        }
    }

    #[test]
    fn every_zone_matches_a_fresh_lookup() {
        // The `tz` column is generated, so this pins the generation to the same lookup the
        // provider falls back to for stationinfo-only stations.
        for station in STATIONS {
            let zone = crate::geo::tz::lookup(station.lat, station.lon);
            assert_eq!(
                zone.map(chrono_tz::Tz::name),
                Some(station.tz),
                "{} coordinates moved?",
                station.icao
            );
        }
    }

    #[test]
    fn lookup_is_case_insensitive_and_exact() {
        let kennedy = lookup("kjfk").expect("KJFK is in the table");
        assert_eq!(kennedy.name, "New York/JF Kennedy Intl");
        assert_eq!(kennedy.state, "NY");
        assert_eq!(kennedy.country, "US");
        assert!(lookup("ZZZZ").is_none());
        assert!(lookup("").is_none());
        // A prefix is not a match: the table answers identifiers, not searches.
        assert!(lookup("KJF").is_none());
    }

    #[test]
    fn nearest_finds_the_closest_row() {
        // Somewhere over Manhattan: the nearest of the fifty-five is Kennedy, not Newark (absent).
        let (station, distance) = nearest_with_distance(40.75, -73.97).expect("a non-empty table");
        assert_eq!(station.icao, "KJFK");
        assert!((distance - 22.0).abs() < 2.0, "{distance} km");
        // And a coordinate sitting on Beijing.
        let station = nearest(39.9042, 116.4074).expect("a non-empty table");
        assert_eq!(station.icao, "ZBAA");
    }
}
