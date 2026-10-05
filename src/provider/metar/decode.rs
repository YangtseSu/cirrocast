// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The METAR/SPECI report decoder: raw text in, canonical metric out.
//!
//! A METAR is a list of space separated groups, and this module turns the subset `cirrocast` can
//! display into [`Decoded`]. It is pure: no clock, no I/O, no cache — the provider supplies the
//! raw report and the observation epoch, and the renderers never see a METAR token.
//!
//! # What is decoded, and by which rule
//!
//! | group | spelling | rule |
//! |---|---|---|
//! | report type | `METAR` / `SPECI` / `COR` | prefix, ignored |
//! | station | `KJFK` | the token before the timestamp; kept as a cross-check |
//! | time | `302351Z` | day of month, hour, minute **UTC** |
//! | wind | `ddd ff KT` | direction (or `VRB`), speed; `00000KT` is calm |
//! | wind (gust) | `ddd ffGfm KT` | gust speed |
//! | wind (metric) | `ddd ff KT` written `MPS` | metres per second, converted to km/h |
//! | wind variation | `270V330` | recorded as `variable_from_deg`/`variable_to_deg` |
//! | visibility | `9999`, `0800` | metres; `9999` is "10 km or more" |
//! | visibility | `10SM`, `P6SM`, `M1/4SM`, `1 3/4SM` | statute miles × 1.609344 |
//! | visibility | `CAVOK` | ≥ 10 km, and no cloud below 5 000 ft |
//! | visibility | `////` | reported as *missing*: `None`, never a number |
//! | RVR | `R28/1200`, `R06L/2000FT` | kept verbatim in [`Decoded::rvr`], never rendered |
//! | weather | `-SHRA`, `BR`, `+TSRA`, `FZRA`, `VCSH` | mapped to a WMO 4677 code |
//! | cloud | `FEW/SCT/BKN/OVC` `nnn` (`CB`/`TCU`) | hundreds of feet → metres |
//! | cloud | `VV006` | vertical visibility, the sky is obscured |
//! | cloud | `NSC`, `NCD`, `CLR`, `SKC` | no cloud |
//! | temperature | `M05/M02` | `M` is minus; °C |
//! | altimeter | `Q1013` | hPa, used as is |
//! | altimeter | `A3010` | inches of mercury × 33.8639 → hPa |
//! | remarks | `RMK …` | **never interpreted**, except the `P####` precipitation group |
//! | recent weather | `RERA`, `RESN`, `RETS`, … | ignored: the weather has ended, it is not the observation |
//! | trend | `NOSIG`, `BECMG`, `TEMPO …` | **never interpreted**: the forecast it carries must not overwrite the observation |
//!
//! # Present weather → WMO 4677
//!
//! One METAR can carry several weather groups (`-RA BR`, `SN FZFG`, `TSRA`); the report's single
//! [`Condition`] is the most significant one, decided in this order:
//!
//! 1. a thunderstorm (`TS…`) — 95, or with hail (`GR`/`GS`) 96 (99 when heavy);
//! 2. freezing precipitation (`FZRA`/`FZDZ`) — 66/67 or 56/57;
//! 3. showery precipitation (`SHRA`/`SHSN`) — 80–82 and 85/86;
//! 4. steady precipitation — rain 61/63/65, snow 71/73/75, drizzle 51/53/55, snow grains 77,
//!    ice pellets and ice crystals 79;
//! 5. an obscuration — fog 45 (rime fog 48), mist 10, haze 5, smoke 4, dust 6, sand 7;
//! 6. nothing: the sky code from the cloud layers — clear 0, few 1, scattered 2, broken and
//!    overcast (and an obscured sky) 3.
//!
//! Intensity is the report's own: a `-` prefix is the light variant, `+` the heavy one, and no
//! prefix is the moderate one, which is also how the WMO codes are laid out. An obscuration this
//! table has no code for (volcanic ash, for example) falls through to rule 6 rather than being
//! clamped into a neighbour: the sky is still what the station reported.
//!
//! # Cloud amount
//!
//! [`Decoded::cloud_cover_pct`] is a project-local percentage derived from the *highest* layer,
//! because a METAR reports coverage per layer and the canonical model carries a single number:
//! `FEW` 20, `SCT` 45, `BKN` 75, `OVC` and an obscured sky 100, no cloud 0. It is a convention, not
//! a measurement, and no renderer pretends otherwise.

use crate::error::{Error, Result};
use crate::model::Condition;

/// Metres per second → km/h.
const MPS_TO_KMH: f32 = 3.6;

/// Knots → km/h.
const KT_TO_KMH: f32 = 1.852;

/// Statute miles → km.
const SM_TO_KM: f32 = 1.609_344;

/// Inches of mercury → hPa.
const INHG_TO_HPA: f32 = 33.8639;

/// Millimetres per inch.
const MM_PER_INCH: f32 = 25.4;

/// `10SM` has no exact metre value; the reported "10 or more" is recorded as 10 km, the value the
/// WMO code 9999 stands for, so the two spellings of "10 km or more" decode identically.
const TEN_KM: f32 = 10.0;

/// Feet → metres.
const FT_TO_M: f64 = 0.3048;

/// One cloud layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudLayer {
    /// The cover token: `FEW`, `SCT`, `BKN`, `OVC` or `VV`.
    pub cover: Cover,
    /// The layer's base in metres above the station, when the report gives one.
    pub base_m: Option<f64>,
    /// Whether the layer is towering cumulus or cumulonimbus.
    pub convective: bool,
}

/// How much of the sky a layer covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    /// `FEW`: 1–2 oktas.
    Few,
    /// `SCT`: 3–4 oktas.
    Scattered,
    /// `BKN`: 5–7 oktas.
    Broken,
    /// `OVC`: 8 oktas.
    Overcast,
    /// `VV`: the sky is obscured and only the vertical visibility is known.
    Obscured,
}

impl Cover {
    /// The current-conditions cloud cover percentage this cover maps to (see the module docs).
    #[must_use]
    pub const fn cover_pct(self) -> u8 {
        match self {
            Self::Few => 20,
            Self::Scattered => 45,
            Self::Broken => 75,
            Self::Overcast | Self::Obscured => 100,
        }
    }

    /// The WMO 4677 sky code this cover maps to when no weather group is present.
    #[must_use]
    pub const fn sky_code(self) -> u8 {
        match self {
            Self::Few => 1,
            Self::Scattered => 2,
            Self::Broken | Self::Overcast | Self::Obscured => 3,
        }
    }
}

/// Everything the decoder reads out of one report, in canonical units.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    /// The station identifier from the report body, when it has one.
    pub icao: Option<String>,
    /// Day of month of the observation (`dd` of `ddHHMMZ`), UTC.
    pub day_of_month: u8,
    /// Hour of the observation, UTC.
    pub hour: u8,
    /// Minute of the observation, UTC.
    pub minute: u8,
    /// Wind direction in degrees, `None` for a variable (`VRB`) wind.
    pub wind_dir_deg: Option<u16>,
    /// Wind speed in km/h.
    pub wind_kmh: f32,
    /// Gust speed in km/h, when the report has a gust.
    pub wind_gust_kmh: Option<f32>,
    /// Whether the report spelled the direction `VRB`.
    pub wind_variable: bool,
    /// Whether the report spelled the wind `00000KT` (calm).
    pub calm: bool,
    /// The start of the reported wind variation sector, when one is given.
    pub variable_from_deg: Option<u16>,
    /// The end of the reported wind variation sector.
    pub variable_to_deg: Option<u16>,
    /// Horizontal visibility in km; `None` when the report does not carry one (`////`).
    pub visibility_km: Option<f32>,
    /// Whether the report carried `CAVOK` (visibility ≥ 10 km, no significant cloud).
    pub cavok: bool,
    /// The present-weather groups, verbatim and in report order.
    pub weather: Vec<String>,
    /// The condition the weather groups (or the sky) map to.
    pub condition: Condition,
    /// Cloud cover percentage derived from the highest layer (see the module docs).
    pub cloud_cover_pct: u8,
    /// The cloud layers, in report order.
    pub cloud_layers: Vec<CloudLayer>,
    /// Air temperature in °C.
    pub temp_c: f32,
    /// Dew point in °C.
    pub dewpoint_c: f32,
    /// Altimeter setting in hPa.
    pub pressure_hpa: f32,
    /// Precipitation in the last hour in mm, from the remark group `P####` (hundredths of an
    /// inch); `None` when the report does not carry one, because most METARs do not.
    pub precip_mm: Option<f32>,
    /// Runway visual range groups, verbatim.
    pub rvr: Vec<String>,
}

/// Applies one group while a trend block (`BECMG`/`TEMPO`) is in force.
///
/// A trend forecast describes expected conditions, not the observation, so nothing it carries may
/// overwrite the report's own groups. The `RMK` marker and its `P####` remark are still read.
fn apply_trend_group(group: &Group, state: &mut State, decoded: &mut Decoded) {
    match group {
        Group::Remarks => state.remarks = true,
        Group::PrecipRemark(mm) => decoded.precip_mm = Some(*mm),
        Group::Trend
        | Group::Wind(_)
        | Group::WindVariation(_, _)
        | Group::Cavok
        | Group::Visibility(_)
        | Group::Rvr
        | Group::Weather(_, _)
        | Group::Cloud(_)
        | Group::Temperature(_, _)
        | Group::Altimeter(_)
        | Group::Ignored => {}
    }
}

/// Decodes one METAR or SPECI report.
///
/// The decoder is strict about the report's *skeleton* — a body with no timestamp or no
/// temperature/dew-point pair is not a METAR this project can display — and lenient about
/// everything else: an unknown group is skipped, not an error, because the group vocabulary is
/// open (remarks, trend forecasts, national extensions) and upstream adds to it without notice.
pub fn decode_metar(raw: &str) -> Result<Decoded> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let time_index = tokens.iter().position(|token| is_timestamp(token));
    let Some(time_index) = time_index else {
        return Err(decode_error(raw, "no ddHHMMZ timestamp"));
    };
    let (day_of_month, hour, minute) = parse_timestamp(tokens[time_index]);
    let icao = time_index
        .checked_sub(1)
        .and_then(|index| tokens.get(index))
        .filter(|token| is_station(token))
        .map(|token| (*token).to_owned());

    let mut decoded = Decoded::empty(icao, (day_of_month, hour, minute));

    let mut state = State::default();
    let mut index = time_index + 1;
    while index < tokens.len() {
        let token = tokens[index];
        index += 1;
        let group = classify(token, state.remarks, tokens.get(index));
        if state.trend {
            apply_trend_group(&group, &mut state, &mut decoded);
            continue;
        }
        match group {
            Group::Trend => state.trend = true,
            Group::Remarks => state.remarks = true,
            Group::PrecipRemark(mm) => decoded.precip_mm = Some(mm),
            Group::Wind(wind) if !state.wind_seen => {
                state.wind_seen = true;
                decoded.wind_dir_deg = wind.direction;
                decoded.wind_kmh = wind.speed_kmh;
                decoded.wind_gust_kmh = wind.gust_kmh;
                decoded.wind_variable = wind.variable;
                decoded.calm = wind.calm;
            }
            Group::WindVariation(from, to) => {
                decoded.variable_from_deg = Some(from);
                decoded.variable_to_deg = Some(to);
            }
            Group::Cavok if !state.visibility_seen => {
                state.visibility_seen = true;
                decoded.cavok = true;
                decoded.visibility_km = Some(TEN_KM);
            }
            Group::Visibility(parsed) if !state.visibility_seen => {
                state.visibility_seen = true;
                decoded.visibility_km = match parsed.value {
                    Visibility::Metres(km) | Visibility::Miles(km) => Some(km),
                    Visibility::Missing => None,
                };
                if parsed.two_token {
                    // `1 3/4SM` is two tokens; the fraction was part of this group.
                    index += 1;
                }
            }
            Group::Rvr => decoded.rvr.push(token.to_owned()),
            Group::Weather(condition, rank) => {
                decoded.weather.push(token.to_owned());
                if rank > state.weather_rank {
                    state.weather_rank = rank;
                    decoded.condition = condition;
                }
            }
            Group::Cloud(layer) => decoded.cloud_layers.push(layer),
            Group::Temperature(temp, dewpoint) if !state.temperature_seen => {
                state.temperature_seen = true;
                decoded.temp_c = temp;
                decoded.dewpoint_c = dewpoint;
            }
            Group::Altimeter(hpa) if !state.pressure_seen => {
                state.pressure_seen = true;
                decoded.pressure_hpa = hpa;
            }
            // Anything else — `AUTO`, `COR`, `NOSIG`, `WS`, national extensions — is not part of
            // the canonical model and is deliberately ignored, and so is a second group of a kind
            // already seen: the first one is the report's own.
            Group::Wind(_)
            | Group::Cavok
            | Group::Visibility(_)
            | Group::Temperature(_, _)
            | Group::Altimeter(_)
            | Group::Ignored => {}
        }
    }

    if !state.temperature_seen {
        return Err(decode_error(raw, "no temperature/dew-point group"));
    }
    if !state.pressure_seen {
        return Err(decode_error(raw, "no altimeter group (Q#### or A####)"));
    }

    decoded.cloud_cover_pct = decoded
        .cloud_layers
        .iter()
        .map(|layer| layer.cover.cover_pct())
        .max()
        .unwrap_or(0);
    if state.weather_rank == 0 {
        decoded.condition = Condition::from_u8(sky_condition(&decoded));
    }
    Ok(decoded)
}

impl Decoded {
    /// The empty decode of a report skeleton: nothing reported yet, in canonical units.
    ///
    /// Every field the decoder has not seen by the end of the pass keeps this value, which is why
    /// a METAR never invents a gust, a visibility or a precipitation amount.
    fn empty(icao: Option<String>, time: (u8, u8, u8)) -> Self {
        let (day_of_month, hour, minute) = time;
        Self {
            icao,
            day_of_month,
            hour,
            minute,
            wind_dir_deg: None,
            wind_kmh: 0.0,
            wind_gust_kmh: None,
            wind_variable: false,
            calm: false,
            variable_from_deg: None,
            variable_to_deg: None,
            visibility_km: None,
            cavok: false,
            weather: Vec::new(),
            condition: Condition::from_u8(0),
            cloud_cover_pct: 0,
            cloud_layers: Vec::new(),
            temp_c: 0.0,
            dewpoint_c: 0.0,
            pressure_hpa: 0.0,
            precip_mm: None,
            rvr: Vec::new(),
        }
    }
}

/// What one decode pass has to remember between groups.
///
/// Four independent facts rather than a bitfield: each one gates a different group, and the
/// decoder reads them by name.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
struct State {
    /// The most significant present-weather group seen so far, as its precedence rank.
    weather_rank: u8,
    /// Whether the wind group has been read (the first one is the report's own).
    wind_seen: bool,
    /// Whether a visibility group (or `CAVOK`) has been read.
    visibility_seen: bool,
    /// Whether a temperature/dew-point group has been seen.
    temperature_seen: bool,
    /// Whether an altimeter group has been seen.
    pressure_seen: bool,
    /// Whether a `BECMG`/`TEMPO` trend marker has been passed; the forecast it introduces is not
    /// the observation and must not overwrite what was already read.
    trend: bool,
    /// Whether the `RMK` marker has been passed; remarks are not interpreted.
    remarks: bool,
}

/// One body group, classified.
enum Group {
    /// `RMK`: the rest of the report is remarks.
    Remarks,
    /// A `P####` remark: precipitation in the last hour, in mm.
    PrecipRemark(f32),
    /// A wind group.
    Wind(Wind),
    /// The direction variation sector `ddd V ddd`.
    WindVariation(u16, u16),
    /// `CAVOK`.
    Cavok,
    /// A visibility group (which may span two tokens).
    Visibility(ParsedVisibility),
    /// A runway visual range group.
    Rvr,
    /// A present-weather group, as its condition and precedence rank.
    Weather(Condition, u8),
    /// A cloud layer.
    Cloud(CloudLayer),
    /// Temperature and dew point.
    Temperature(f32, f32),
    /// The altimeter setting in hPa.
    Altimeter(f32),
    /// A `BECMG`/`TEMPO` trend marker: the groups after it are a forecast, not the observation.
    Trend,
    /// Anything the canonical model does not carry.
    Ignored,
}

/// Classifies one token, in the order the groups can be told apart.
///
/// `remarks` says whether the `RMK` marker has already been passed, which changes what a token can
/// mean: after it, only `P####` is read.
fn classify(token: &str, remarks: bool, next: Option<&&str>) -> Group {
    if token == "RMK" {
        return Group::Remarks;
    }
    if remarks {
        return parse_precip_remark(token).map_or(Group::Ignored, Group::PrecipRemark);
    }
    if token == "BECMG" || token == "TEMPO" {
        return Group::Trend;
    }
    if let Some(wind) = parse_wind(token) {
        return Group::Wind(wind);
    }
    if let Some((from, to)) = parse_wind_variation(token) {
        return Group::WindVariation(from, to);
    }
    if token == "CAVOK" {
        return Group::Cavok;
    }
    if let Some(parsed) = parse_visibility(token, next) {
        return Group::Visibility(parsed);
    }
    if is_rvr(token) {
        return Group::Rvr;
    }
    if let Some((condition, rank)) = parse_weather(token) {
        return Group::Weather(condition, rank);
    }
    if let Some(layer) = parse_cloud(token) {
        return Group::Cloud(layer);
    }
    if let Some((temp, dewpoint)) = parse_temperature(token) {
        return Group::Temperature(temp, dewpoint);
    }
    if let Some(hpa) = parse_altimeter(token) {
        return Group::Altimeter(hpa);
    }
    Group::Ignored
}

/// The one-line error every malformed report produces: the report itself is the context, because
/// it is what a bug report has to quote.
fn decode_error(raw: &str, reason: &str) -> Error {
    Error::Upstream {
        provider: "metar".to_owned(),
        status: None,
        message: format!("cannot decode the METAR report `{raw}`: {reason}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Token parsing
// ---------------------------------------------------------------------------------------------

/// `302351Z`: day of month, hour and minute, UTC.
fn is_timestamp(token: &str) -> bool {
    token.len() == 7 && token.ends_with('Z') && token[..6].bytes().all(|byte| byte.is_ascii_digit())
}

/// The three numbers of a timestamp; the token is already known to be one.
fn parse_timestamp(token: &str) -> (u8, u8, u8) {
    let number = |range: std::ops::Range<usize>| {
        token
            .get(range)
            .and_then(|text| text.parse::<u8>().ok())
            .unwrap_or(0)
    };
    (number(0..2), number(2..4), number(4..6))
}

/// A station identifier: exactly four upper-case letters or digits, at least one letter.
fn is_station(token: &str) -> bool {
    token.len() == 4
        && token
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && token.bytes().any(|byte| byte.is_ascii_uppercase())
}

/// One wind group.
struct Wind {
    direction: Option<u16>,
    speed_kmh: f32,
    gust_kmh: Option<f32>,
    variable: bool,
    calm: bool,
}

/// `ddd ff KT`, `ddd ffGfm KT`, `VRB ff KT`, `00000KT`, with `KT` or `MPS` as the unit.
fn parse_wind(token: &str) -> Option<Wind> {
    let (body, to_kmh) = token.strip_suffix("KT").map_or_else(
        || token.strip_suffix("MPS").map(|body| (body, MPS_TO_KMH)),
        |body| Some((body, KT_TO_KMH)),
    )?;

    let (direction, speed) = if let Some(speed) = body.strip_prefix("VRB") {
        (None, speed)
    } else {
        let direction: u16 = body.get(..3)?.parse().ok()?;
        (Some(direction), body.get(3..)?)
    };
    let (speed, gust) = match speed.split_once('G') {
        Some((speed, gust)) => (speed, Some(gust)),
        None => (speed, None),
    };
    let speed_kmh = speed.parse::<f32>().ok()? * to_kmh;
    let gust_kmh = gust.and_then(|gust| gust.parse::<f32>().ok().map(|value| value * to_kmh));

    Some(Wind {
        direction,
        speed_kmh,
        gust_kmh,
        variable: direction.is_none(),
        calm: direction.is_some_and(|direction| direction == 0) && speed_kmh.abs() < f32::EPSILON,
    })
}

/// `270V330`: the sector the wind direction varies over.
fn parse_wind_variation(token: &str) -> Option<(u16, u16)> {
    let (from, to) = token.split_once('V')?;
    if from.len() != 3 || to.len() != 3 {
        return None;
    }
    Some((from.parse().ok()?, to.parse().ok()?))
}

/// One visibility group, already classified.
enum Visibility {
    /// A metric group (`0800`, `9999`), converted to km.
    Metres(f32),
    /// A statute-mile group (`10SM`, `P6SM`, `M1/4SM`, `1 3/4SM`), converted to km.
    Miles(f32),
    /// `////`: the report occupies the visibility slot but has no value for it.
    Missing,
}

/// A parsed visibility group and whether it swallowed the next token.
struct ParsedVisibility {
    /// The value.
    value: Visibility,
    /// Whether the following token was part of the group (`1 3/4SM`).
    two_token: bool,
}

/// A visibility group. `next` is the following token, needed for the `1 3/4SM` spelling.
fn parse_visibility(token: &str, next: Option<&&str>) -> Option<ParsedVisibility> {
    let single = |value| {
        Some(ParsedVisibility {
            value,
            two_token: false,
        })
    };
    if token == "////" {
        return single(Visibility::Missing);
    }
    if token.len() == 4 && token.bytes().all(|byte| byte.is_ascii_digit()) {
        // Four digits are metres; 9999 is the "10 km or more" sentinel.
        let metres: f32 = token.parse().ok()?;
        return single(Visibility::Metres(if metres >= 9999.0 {
            TEN_KM
        } else {
            metres / 1000.0
        }));
    }
    if let Some(km) = parse_miles(token) {
        return single(Visibility::Miles(km));
    }
    // `1 3/4SM` is two tokens: a whole number of miles and a fraction group.
    if token.len() <= 2
        && token.bytes().all(|byte| byte.is_ascii_digit())
        && let Some(fraction) = next.and_then(|next| next.strip_suffix("SM"))
        && let Some(fraction) = parse_miles_fraction(fraction)
    {
        let whole: f32 = token.parse().ok()?;
        return Some(ParsedVisibility {
            value: Visibility::Miles((whole + fraction) * SM_TO_KM),
            two_token: true,
        });
    }
    None
}

/// A statute-mile visibility group: `10SM`, `P6SM` (more than 6), `M1/4SM` (less than a quarter).
fn parse_miles(token: &str) -> Option<f32> {
    let body = token.strip_suffix("SM")?;
    if let Some(over) = body.strip_prefix('P') {
        // `P6SM` is the ceiling of the scale; 6 miles is the value the report pins.
        return over.parse::<f32>().ok().map(|miles| miles * SM_TO_KM);
    }
    if let Some(under) = body.strip_prefix('M') {
        return parse_miles_fraction(under).map(|miles| miles * SM_TO_KM);
    }
    if let Some(fraction) = parse_miles_fraction(body) {
        return Some(fraction * SM_TO_KM);
    }
    body.parse::<f32>().ok().map(|miles| miles * SM_TO_KM)
}

/// `3/4` → 0.75; `10` → `None`, because a whole number is not a fraction group.
fn parse_miles_fraction(text: &str) -> Option<f32> {
    let (numerator, denominator) = text.split_once('/')?;
    let numerator: f32 = numerator.parse().ok()?;
    let denominator: f32 = denominator.parse().ok()?;
    if denominator == 0.0 {
        return None;
    }
    Some(numerator / denominator)
}

/// RVR groups: `R28/1200`, `R06L/2000FT`, `R06/2600V3500FT`.
fn is_rvr(token: &str) -> bool {
    let Some(rest) = token.strip_prefix('R') else {
        return false;
    };
    let Some((runway, value)) = rest.split_once('/') else {
        return false;
    };
    // The runway is two digits, optionally followed by the parallel-runway letter (`06L`, `24R`,
    // `06C`); the all-digit form without a letter is common too.
    let designator = match runway.as_bytes() {
        [first, second] => first.is_ascii_digit() && second.is_ascii_digit(),
        [first, second, letter] => {
            first.is_ascii_digit()
                && second.is_ascii_digit()
                && matches!(letter, b'L' | b'C' | b'R')
        }
        _ => false,
    };
    designator
        && !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'V' | b'P' | b'M' | b'F' | b'T' | b'L' | b'R')
        })
}

/// One cloud layer: `FEW045`, `BKN250CB`, `VV006`, `OVC///`.
fn parse_cloud(token: &str) -> Option<CloudLayer> {
    let (cover, rest) = if let Some(rest) = token.strip_prefix("VV") {
        (Cover::Obscured, rest)
    } else if let Some(rest) = token.strip_prefix("FEW") {
        (Cover::Few, rest)
    } else if let Some(rest) = token.strip_prefix("SCT") {
        (Cover::Scattered, rest)
    } else if let Some(rest) = token.strip_prefix("BKN") {
        (Cover::Broken, rest)
    } else {
        (Cover::Overcast, token.strip_prefix("OVC")?)
    };

    let (height, convection) = match rest.strip_suffix("TCU") {
        Some(height) => (height, true),
        None => match rest.strip_suffix("CB") {
            Some(height) => (height, true),
            None => (rest, false),
        },
    };
    // `///` (or any non-numeric height) means the base is not reported.
    let base_m = height
        .parse::<f64>()
        .ok()
        .map(|hundreds_of_feet| hundreds_of_feet * 100.0 * FT_TO_M);
    Some(CloudLayer {
        cover,
        base_m,
        convective: convection,
    })
}

/// `M05/M02` or `18/16`: temperature and dew point, `M` for minus.
fn parse_temperature(token: &str) -> Option<(f32, f32)> {
    let (temperature, dewpoint) = token.split_once('/')?;
    Some((parse_signed(temperature)?, parse_signed(dewpoint)?))
}

/// A signed two-or-three digit Celsius value (`M05`, `05`, `2`, `M10`).
fn parse_signed(token: &str) -> Option<f32> {
    let (negative, digits) = match token.strip_prefix('M') {
        Some(digits) => (true, digits),
        None => (false, token),
    };
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: f32 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// `Q1013` (hPa) or `A3010` (inches of mercury).
fn parse_altimeter(token: &str) -> Option<f32> {
    if let Some(hpa) = token.strip_prefix('Q') {
        if hpa.len() == 4 && hpa.bytes().all(|byte| byte.is_ascii_digit()) {
            return hpa.parse::<f32>().ok();
        }
        return None;
    }
    let inches = token.strip_prefix('A')?;
    if inches.len() != 4 || !inches.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: f32 = inches.parse().ok()?;
    Some(value / 100.0 * INHG_TO_HPA)
}

/// `P0000` in the remarks: precipitation since the last report, hundredths of an inch.
fn parse_precip_remark(token: &str) -> Option<f32> {
    let digits = token.strip_prefix('P')?;
    if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let hundredths: f32 = digits.parse().ok()?;
    Some(hundredths / 100.0 * MM_PER_INCH)
}

/// One present-weather group as WMO 4677 and the precedence rank that decides which group of a
/// report becomes the single [`Condition`] (higher wins; see the module docs).
///
/// `None` means the token is not a weather group at all — `AUTO`, `NOSIG` or a national extension
/// must not be mistaken for one.
fn parse_weather(token: &str) -> Option<(Condition, u8)> {
    let (intensity, rest) = match token.chars().next()? {
        '-' => (Intensity::Light, token.get(1..)?),
        '+' => (Intensity::Heavy, token.get(1..)?),
        _ => (Intensity::Moderate, token),
    };
    let rest = rest.strip_prefix("VC").unwrap_or(rest);
    let mut remaining = rest;
    let mut descriptor = None;
    for candidate in WEATHER_DESCRIPTORS {
        if let Some(rest) = remaining.strip_prefix(candidate) {
            descriptor = Some(candidate);
            remaining = rest;
            break;
        }
    }

    // The remaining codes, longest first so `TSRA` cannot read as `TS` + `R` + `A`.
    let mut precip = Vec::new();
    let mut obscuration = Vec::new();
    let mut rest = remaining;
    while !rest.is_empty() {
        let matched = PRECIP_CODES
            .iter()
            .chain(OBSCURATION_CODES.iter())
            .find(|code| rest.starts_with(**code))?;
        if PRECIP_CODES.contains(matched) {
            precip.push(*matched);
        } else {
            obscuration.push(*matched);
        }
        rest = &rest[matched.len()..];
    }
    if precip.is_empty() && obscuration.is_empty() {
        // `VCSH`: showers in the vicinity with no precipitation type named. The shower family's
        // light member is the described code the token maps to.
        if descriptor == Some("SH") {
            return Some((Condition::from_u8(80), RANK_PRECIPITATION));
        }
        descriptor?;
    }

    if descriptor == Some("TS") {
        let hail = precip.iter().any(|code| *code == "GR" || *code == "GS");
        let condition = match (hail, intensity) {
            (true, Intensity::Heavy) => 99,
            (true, _) => 96,
            (false, _) => 95,
        };
        return Some((Condition::from_u8(condition), RANK_THUNDER));
    }
    if descriptor == Some("FZ") {
        if precip.contains(&"RA") {
            let condition = match intensity {
                Intensity::Light => 66,
                Intensity::Moderate | Intensity::Heavy => 67,
            };
            return Some((Condition::from_u8(condition), RANK_FREEZING));
        }
        if precip.contains(&"DZ") {
            let condition = match intensity {
                Intensity::Light => 56,
                Intensity::Heavy | Intensity::Moderate => 57,
            };
            return Some((Condition::from_u8(condition), RANK_FREEZING));
        }
    }
    if let Some(code) = precip.first() {
        return Some((
            Condition::from_u8(precip_code(code, descriptor == Some("SH"), intensity)),
            RANK_PRECIPITATION,
        ));
    }
    let code = obscuration.first()?;
    let wmo = obscuration_code(code, descriptor == Some("FZ"));
    // An obscuration the table does not name (volcanic ash, for example) is not a condition this
    // model can describe: the group is treated as no weather, so the sky code decides (rule 6 in
    // the module docs) instead of reporting an undescribed code that renders as "Clear sky".
    if wmo == 0 {
        return None;
    }
    Some((Condition::from_u8(wmo), RANK_OBSCURATION))
}

/// A thunderstorm outranks everything else in the report.
const RANK_THUNDER: u8 = 4;
/// Freezing precipitation outranks its unfrozen equivalent.
const RANK_FREEZING: u8 = 3;
/// Steady and showery precipitation outranks an obscuration.
const RANK_PRECIPITATION: u8 = 2;
/// An obscuration outranks the sky code it hides.
const RANK_OBSCURATION: u8 = 1;

/// How hard the report says the phenomenon is falling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intensity {
    /// The `-` prefix.
    Light,
    /// No prefix.
    Moderate,
    /// The `+` prefix.
    Heavy,
}

/// The precipitation codes a METAR can carry, in match order.
const PRECIP_CODES: [&str; 8] = ["DZ", "RA", "SN", "SG", "IC", "PL", "GR", "GS"];

/// The obscuration codes a METAR can carry, in match order.
const OBSCURATION_CODES: [&str; 7] = ["BR", "FG", "FU", "VA", "DU", "SA", "HZ"];

/// The descriptors this decoder understands; anything else means the token is not a weather group.
const WEATHER_DESCRIPTORS: [&str; 8] = ["MI", "PR", "BC", "DR", "BL", "SH", "TS", "FZ"];

/// A precipitation code as a WMO 4677 code.
fn precip_code(code: &str, showery: bool, intensity: Intensity) -> u8 {
    match code {
        "DZ" => match intensity {
            Intensity::Light => 51,
            Intensity::Moderate => 53,
            Intensity::Heavy => 55,
        },
        "RA" if showery => match intensity {
            Intensity::Light => 80,
            Intensity::Moderate => 81,
            Intensity::Heavy => 82,
        },
        "RA" => match intensity {
            Intensity::Light => 61,
            Intensity::Moderate => 63,
            Intensity::Heavy => 65,
        },
        "SN" if showery => match intensity {
            Intensity::Light => 85,
            Intensity::Moderate | Intensity::Heavy => 86,
        },
        "SN" => match intensity {
            Intensity::Light => 71,
            Intensity::Moderate => 73,
            Intensity::Heavy => 75,
        },
        "SG" => 77,
        // Ice crystals (IC) have no described code in this model's WMO 4677 table (76 is absent),
        // so they share ice pellets' code: the nearest described family that keeps the observation
        // renderable as a real condition.
        "PL" | "IC" => 79,
        // Hail and small hail have no non-thunderstorm code in WMO 4677; the report carries hail,
        // so the hail code is the honest answer, and a thunderstorm already took the branch above.
        "GR" | "GS" => match intensity {
            Intensity::Heavy => 99,
            Intensity::Light | Intensity::Moderate => 96,
        },
        _ => 0,
    }
}

/// An obscuration code as a WMO 4677 code.
fn obscuration_code(code: &str, freezing: bool) -> u8 {
    match code {
        "FG" if freezing => 48,
        "FG" => 45,
        "BR" => 10,
        "HZ" => 5,
        "FU" => 4,
        "DU" => 6,
        "SA" => 7,
        // Volcanic ash has no code in the described set; the sky decides instead.
        _ => 0,
    }
}

/// The sky code of a report with no present weather: the highest layer decides.
fn sky_condition(decoded: &Decoded) -> u8 {
    decoded
        .cloud_layers
        .iter()
        .map(|layer| layer.cover.sky_code())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Cover, decode_metar, is_rvr, parse_miles, parse_weather, parse_wind};
    use crate::model::Condition;

    #[test]
    fn ice_crystals_map_to_a_described_condition() {
        let (condition, _) = parse_weather("IC").expect("ice crystals are a weather group");
        assert_eq!(condition, Condition::from_u8(79));
        assert!(condition.is_known(), "79 must be described by the catalog");
    }

    #[test]
    fn volcanic_ash_leaves_the_condition_to_the_sky() {
        // `VA` has no described code; the report must fall through to the sky layers, which are
        // broken here, rather than report something that renders as "Clear sky".
        let decoded = decode_metar("METAR KJFK 302351Z 00000KT 10SM VA BKN020 18/16 A3010")
            .expect("a complete report");
        assert_eq!(decoded.condition, Condition::from_u8(3));
        assert_ne!(decoded.condition.description_en(), "Clear sky");
        assert!(
            decoded.weather.is_empty(),
            "VA is not a described condition"
        );
    }

    #[test]
    fn a_wind_speed_keeps_its_exact_conversion_for_the_renderer_to_round() {
        let wind = parse_wind("29017KT").expect("a wind group");
        assert!((wind.speed_kmh - 31.484).abs() < 1e-3, "{}", wind.speed_kmh);
        assert!(
            (wind.speed_kmh - 31.5).abs() > 1e-3,
            "the decoder must not round: {}",
            wind.speed_kmh
        );
    }

    #[test]
    fn wind_groups_convert_to_kmh() {
        let wind = parse_wind("14004KT").expect("a wind group");
        assert_eq!(wind.direction, Some(140));
        assert!((wind.speed_kmh - 7.4).abs() < 0.05, "{}", wind.speed_kmh);
        assert_eq!(wind.gust_kmh, None);
        assert!(!wind.calm);

        let gusting = parse_wind("29005G10MPS").expect("a metric wind group");
        assert_eq!(gusting.direction, Some(290));
        assert!(
            (gusting.speed_kmh - 18.0).abs() < 0.05,
            "{}",
            gusting.speed_kmh
        );
        assert!((gusting.gust_kmh.expect("a gust") - 36.0).abs() < 0.05);

        let variable = parse_wind("VRB02KT").expect("a variable wind");
        assert_eq!(variable.direction, None);
        assert!(variable.variable);

        let calm = parse_wind("00000KT").expect("a calm wind");
        assert!(calm.calm);
        assert_eq!(calm.direction, Some(0));
        assert!((calm.speed_kmh).abs() < f32::EPSILON);
    }

    #[test]
    fn visibility_spellings_convert_to_km() {
        assert_eq!(
            parse_miles("10SM").map(|km| (km * 100.0).round()),
            Some(1609.0)
        );
        assert_eq!(
            parse_miles("1/4SM").map(|km| (km * 100.0).round()),
            Some(40.0)
        );
        assert_eq!(
            parse_miles("P6SM").map(|km| (km * 100.0).round()),
            Some(966.0)
        );
        assert_eq!(
            parse_miles("M1/4SM").map(|km| (km * 100.0).round()),
            Some(40.0)
        );
        assert!(parse_miles("KT").is_none());
    }

    #[test]
    fn a_report_without_a_body_group_is_an_upstream_error() {
        let error = decode_metar("METAR KJFK").unwrap_err();
        assert_eq!(error.exit_code(), 3);
        assert!(
            error.to_string().contains("no ddHHMMZ timestamp"),
            "{error}"
        );
    }

    #[test]
    fn cloud_covers_map_to_percentages() {
        assert_eq!(Cover::Few.cover_pct(), 20);
        assert_eq!(Cover::Scattered.cover_pct(), 45);
        assert_eq!(Cover::Broken.cover_pct(), 75);
        assert_eq!(Cover::Overcast.cover_pct(), 100);
    }

    #[test]
    fn a_trend_block_does_not_overwrite_the_observation() {
        // The observation is 10 km visibility, light rain, 18/16, Q1018; the `TEMPO` that follows
        // is a forecast and must not replace any of it.
        let decoded = decode_metar(
            "METAR ZBAA 010000Z 00000KT 9999 -RA BKN016 18/16 Q1018 TEMPO 0600 TSRA 15/14",
        )
        .expect("a complete report");
        assert_eq!(decoded.visibility_km, Some(10.0));
        assert_eq!(decoded.temp_c, 18.0);
        assert_eq!(decoded.dewpoint_c, 16.0);
        assert_eq!(decoded.pressure_hpa, 1018.0);
        assert_eq!(decoded.condition, Condition::from_u8(61));
    }

    #[test]
    fn a_becmg_block_is_ignored_but_remarks_after_it_are_read() {
        let decoded =
            decode_metar("METAR ZBAA 010000Z 24008KT 9999 18/16 Q1018 BECMG 12/11 RMK P0000")
                .expect("a complete report");
        assert_eq!(decoded.temp_c, 18.0);
        assert_eq!(decoded.dewpoint_c, 16.0);
        assert_eq!(decoded.precip_mm, Some(0.0));
    }

    #[test]
    fn an_rvr_may_carry_a_parallel_runway_designator() {
        assert!(is_rvr("R06L/2000FT"));
        assert!(is_rvr("R28/1200"));
        assert!(is_rvr("R06/2600V3500FT"));
        assert!(!is_rvr("R2/1200"));
        assert!(!is_rvr("R28/"));
        let decoded = decode_metar("METAR ZBAA 010000Z 24008KT 9999 R06L/2000FT 18/16 Q1018")
            .expect("a complete report");
        assert_eq!(decoded.rvr, vec!["R06L/2000FT".to_owned()]);
    }

    #[test]
    fn vicinity_showers_map_into_the_shower_family() {
        let (condition, rank) = parse_weather("VCSH").expect("VCSH is a weather group");
        assert_eq!(condition, Condition::from_u8(80));
        assert!(condition.is_known());
        assert_eq!(rank, super::RANK_PRECIPITATION);
    }
}
