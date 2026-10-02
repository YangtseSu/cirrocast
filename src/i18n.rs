// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Language selection and the message lookup every renderer goes through.
//!
//! A renderer never writes a user-visible word itself: it asks [`I18n`] for a key, and the key is
//! what a catalog is written against. The catalogs are Fluent files under `locales/`, embedded in
//! the binary with `include_str!` — there is no runtime catalog directory to find, and a missing
//! translation is a failing test (`tests/i18n.rs`), not a silent English string in the middle of
//! Chinese output.
//!
//! Three things are deliberately not the render layer's business, and this module is where they
//! live instead:
//!
//! * **Negotiation.** `--lang` wins, then `LC_ALL`, `LC_MESSAGES` and `LANG` in that order, then
//!   `en-US`. `zh_CN.UTF-8` normalises to `zh-CN`; an `en-*` or `zh-*` tag resolves through the
//!   explicit chain for its family — `en-GB → en-US`, `zh-TW → zh-CN → en-US` — without a warning,
//!   because the substitution is the documented answer, not a surprise. A family with no catalog
//!   (`fr`, `de-DE`) falls back to `en-US` and says so.
//! * **Completeness.** [`RENDERER_KEYS`] is the single list of every static key renderers may ask
//!   for, and the [`keys`] constants are the only way the code names them. A catalog that lacks a
//!   key is caught by the completeness test, so a new label cannot ship untranslated.
//! * **Formatting.** Numbers and dates go through messages (`format-*`, `date-*`) that take an
//!   already-converted value, so no locale-dependent C formatting (a comma decimal separator, a
//!   translated month name from `chrono`) can leak into the output.
//!
//! The one piece of interior state is the note list: a lookup that fails has to be able to say so
//! without `&mut self`, because it is reached through `&RenderContext` like every other renderer
//! call.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;

use chrono::{Datelike as _, NaiveDate};
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

use crate::model::DayPartKind;
use crate::model::condition::Condition;

/// The catalogs embedded in the binary: `(tag, source)`, in the order the completeness test and
/// `reuse lint` see them.
///
/// Adding a language is one line here plus the file itself — no build script, no code change in a
/// renderer. `.ftl` files carry `#` comments natively, so each file holds its own SPDX header.
pub const CATALOGS: &[(&str, &str)] = &[
    ("en-US", include_str!("../locales/en-US/main.ftl")),
    ("zh-CN", include_str!("../locales/zh-CN/main.ftl")),
];

/// The language every other language falls back to; its catalog must be complete.
pub const DEFAULT_TAG: &str = "en-US";

/// Where a region-less `zh` region falls back to; see [`language_chain`].
const CHINESE_FALLBACK: &str = "zh-CN";

// ---------------------------------------------------------------------------------------------
// Message keys
// ---------------------------------------------------------------------------------------------

/// A catalog key, as the renderer-side constants spell it.
///
/// A newtype rather than a bare `&str` so a renderer cannot invent a key at a call site: the
/// [`keys`] constants and the key functions of this module are the only ways to build one. The
/// hundred condition keys are derived from the code rather than listed, which is the one case where
/// a key is not a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageKey(KeySource);

/// Where a key's spelling comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeySource {
    /// One of the [`keys`] constants.
    Static(&'static str),
    /// `cond-<code>`, built from a condition code.
    Condition(u8),
}

impl MessageKey {
    /// Wraps a literal key. Not public: the key set is [`keys`].
    const fn new(key: &'static str) -> Self {
        Self(KeySource::Static(key))
    }

    /// The key, as it appears in a `.ftl` file.
    #[must_use]
    pub fn as_str(&self) -> Cow<'_, str> {
        match &self.0 {
            KeySource::Static(key) => Cow::Borrowed(key),
            KeySource::Condition(code) => Cow::Owned(format!("cond-{code}")),
        }
    }
}

impl fmt::Display for MessageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_str())
    }
}

/// Every static catalog key the renderers, the `one-line` tokens and the CLI ask for.
///
/// Grouped as the catalogs are. The sets that carry a number — the four day parts, the seven
/// weekdays, the twelve months, the sixteen compass points and the five UV bands — are arrays here
/// and are picked by the matching function of this module (`day_part_key`, `weekday_key`, …); the
/// condition keys are derived from the code by `condition_key` and their spellings are covered by
/// the completeness test's own enumeration of `0..=99`.
pub mod keys {
    use super::MessageKey;

    /// The report heading (`Weather report:`).
    pub const LABEL_REPORT: MessageKey = MessageKey::new("label-report");
    /// The data attribution's own label (`Data:`).
    pub const LABEL_DATA: MessageKey = MessageKey::new("label-data");
    /// The `plain` record key for the place (`location`).
    pub const LABEL_LOCATION: MessageKey = MessageKey::new("label-location");
    /// The `plain` record key for the observation time (`updated`).
    pub const LABEL_UPDATED: MessageKey = MessageKey::new("label-updated");
    /// The `plain` record key for the current conditions (`current`).
    pub const LABEL_CURRENT: MessageKey = MessageKey::new("label-current");
    /// The `plain` record key prefixing a forecast day (`day`).
    pub const LABEL_DAY: MessageKey = MessageKey::new("label-day");
    /// The `plain` record key for the provider and its endpoint (`attribution`).
    pub const LABEL_ATTRIBUTION: MessageKey = MessageKey::new("label-attribution");
    /// The apparent-temperature label (`feels`).
    pub const LABEL_FEELS: MessageKey = MessageKey::new("label-feels");
    /// The wind label.
    pub const LABEL_WIND: MessageKey = MessageKey::new("label-wind");
    /// The humidity label.
    pub const LABEL_HUMIDITY: MessageKey = MessageKey::new("label-humidity");
    /// The precipitation label.
    pub const LABEL_PRECIP: MessageKey = MessageKey::new("label-precip");
    /// The pressure label.
    pub const LABEL_PRESSURE: MessageKey = MessageKey::new("label-pressure");
    /// The visibility label.
    pub const LABEL_VISIBILITY: MessageKey = MessageKey::new("label-visibility");
    /// The UV-index label.
    pub const LABEL_UV: MessageKey = MessageKey::new("label-uv");
    /// The sunrise label.
    pub const LABEL_SUNRISE: MessageKey = MessageKey::new("label-sunrise");
    /// The sunset label.
    pub const LABEL_SUNSET: MessageKey = MessageKey::new("label-sunset");
    /// The observation line's label (`observed`).
    pub const LABEL_OBSERVED: MessageKey = MessageKey::new("label-observed");
    /// The age of an observation in minutes, for the observation line.
    pub const FORMAT_AGE_MINUTES: MessageKey = MessageKey::new("format-age-minutes");
    /// The age of an observation in hours, for the observation line.
    pub const FORMAT_AGE_HOURS: MessageKey = MessageKey::new("format-age-hours");
    /// The footer of an observation-only report (`no forecast: METAR is an observation`).
    pub const NOTE_NO_FORECAST: MessageKey = MessageKey::new("note-no-forecast");

    /// `n/a`, for a value the provider does not report.
    pub const NA: MessageKey = MessageKey::new("na");
    /// `n/a` for the moon phase, separate so the two can drift apart.
    pub const MOON_NA: MessageKey = MessageKey::new("moon-na");

    /// Temperature in Celsius.
    pub const FORMAT_TEMP_C: MessageKey = MessageKey::new("format-temp-c");
    /// Temperature in Fahrenheit.
    pub const FORMAT_TEMP_F: MessageKey = MessageKey::new("format-temp-f");
    /// Wind in km/h.
    pub const FORMAT_WIND_KMH: MessageKey = MessageKey::new("format-wind-kmh");
    /// Wind in mph.
    pub const FORMAT_WIND_MPH: MessageKey = MessageKey::new("format-wind-mph");
    /// Wind in knots.
    pub const FORMAT_WIND_KNOTS: MessageKey = MessageKey::new("format-wind-knots");
    /// Wind in m/s.
    pub const FORMAT_WIND_MPS: MessageKey = MessageKey::new("format-wind-mps");
    /// Pressure in hPa.
    pub const FORMAT_PRESSURE_HPA: MessageKey = MessageKey::new("format-pressure-hpa");
    /// Pressure in inHg.
    pub const FORMAT_PRESSURE_INHG: MessageKey = MessageKey::new("format-pressure-inhg");
    /// Pressure in mmHg.
    pub const FORMAT_PRESSURE_MMHG: MessageKey = MessageKey::new("format-pressure-mmhg");
    /// Distance in kilometres.
    pub const FORMAT_DISTANCE_KM: MessageKey = MessageKey::new("format-distance-km");
    /// Distance in miles.
    pub const FORMAT_DISTANCE_MI: MessageKey = MessageKey::new("format-distance-mi");
    /// The humidity percentage suffix.
    pub const FORMAT_HUMIDITY: MessageKey = MessageKey::new("format-humidity");
    /// The UV index with its name, for `%U`.
    pub const FORMAT_UV: MessageKey = MessageKey::new("format-uv");
    /// Precipitation in millimetres.
    pub const FORMAT_PRECIP_MM: MessageKey = MessageKey::new("format-precip-mm");
    /// Precipitation in inches.
    pub const FORMAT_PRECIP_IN: MessageKey = MessageKey::new("format-precip-in");

    /// The date styles, by [`super::DateStyle`].
    pub const DATE_ISO: MessageKey = MessageKey::new("date-iso");
    /// The short date style.
    pub const DATE_SHORT: MessageKey = MessageKey::new("date-short");
    /// The today date style.
    pub const DATE_TODAY: MessageKey = MessageKey::new("date-today");

    /// The four day-part labels, in calendar order.
    pub const PARTS: [MessageKey; 4] = [
        MessageKey::new("part-morning"),
        MessageKey::new("part-noon"),
        MessageKey::new("part-evening"),
        MessageKey::new("part-night"),
    ];
    /// The seven weekday names, Monday first.
    pub const WEEKDAYS: [MessageKey; 7] = [
        MessageKey::new("weekday-mon"),
        MessageKey::new("weekday-tue"),
        MessageKey::new("weekday-wed"),
        MessageKey::new("weekday-thu"),
        MessageKey::new("weekday-fri"),
        MessageKey::new("weekday-sat"),
        MessageKey::new("weekday-sun"),
    ];
    /// The twelve month names, January first.
    pub const MONTHS: [MessageKey; 12] = [
        MessageKey::new("month-1"),
        MessageKey::new("month-2"),
        MessageKey::new("month-3"),
        MessageKey::new("month-4"),
        MessageKey::new("month-5"),
        MessageKey::new("month-6"),
        MessageKey::new("month-7"),
        MessageKey::new("month-8"),
        MessageKey::new("month-9"),
        MessageKey::new("month-10"),
        MessageKey::new("month-11"),
        MessageKey::new("month-12"),
    ];
    /// The five UV bands, weakest first.
    pub const UV_BANDS: [MessageKey; 5] = [
        MessageKey::new("uv-band-low"),
        MessageKey::new("uv-band-moderate"),
        MessageKey::new("uv-band-high"),
        MessageKey::new("uv-band-very-high"),
        MessageKey::new("uv-band-extreme"),
    ];
    /// The sixteen compass points, clockwise from north.
    pub const DIRECTIONS: [MessageKey; 16] = [
        MessageKey::new("dir-n"),
        MessageKey::new("dir-nne"),
        MessageKey::new("dir-ne"),
        MessageKey::new("dir-ene"),
        MessageKey::new("dir-e"),
        MessageKey::new("dir-ese"),
        MessageKey::new("dir-se"),
        MessageKey::new("dir-sse"),
        MessageKey::new("dir-s"),
        MessageKey::new("dir-ssw"),
        MessageKey::new("dir-sw"),
        MessageKey::new("dir-wsw"),
        MessageKey::new("dir-w"),
        MessageKey::new("dir-wnw"),
        MessageKey::new("dir-nw"),
        MessageKey::new("dir-nnw"),
    ];
    /// The condition of a code no WMO row describes.
    pub const CONDITION_UNKNOWN: MessageKey = MessageKey::new("cond-unknown");
}

/// Every static message key a renderer, a token or the CLI can ask for, in one list.
///
/// The completeness test walks this over every catalog, so a key that exists in no catalog — or in
/// only one of them — fails the build. A few of them (`label-uv`, `label-sunrise`, `label-sunset`,
/// `format-distance-*`, `format-wind-knots`, …) are listed before a renderer asks for them: the
/// step that wants one then has a translation to use instead of a label to invent, and the
/// catalogs stay complete in the meantime.
pub const RENDERER_KEYS: &[MessageKey] = &[
    keys::LABEL_REPORT,
    keys::LABEL_DATA,
    keys::LABEL_LOCATION,
    keys::LABEL_UPDATED,
    keys::LABEL_CURRENT,
    keys::LABEL_DAY,
    keys::LABEL_ATTRIBUTION,
    keys::LABEL_FEELS,
    keys::LABEL_WIND,
    keys::LABEL_HUMIDITY,
    keys::LABEL_PRECIP,
    keys::LABEL_PRESSURE,
    keys::LABEL_VISIBILITY,
    keys::LABEL_UV,
    keys::LABEL_SUNRISE,
    keys::LABEL_SUNSET,
    keys::LABEL_OBSERVED,
    keys::FORMAT_AGE_MINUTES,
    keys::FORMAT_AGE_HOURS,
    keys::NOTE_NO_FORECAST,
    keys::NA,
    keys::MOON_NA,
    keys::FORMAT_TEMP_C,
    keys::FORMAT_TEMP_F,
    keys::FORMAT_WIND_KMH,
    keys::FORMAT_WIND_MPH,
    keys::FORMAT_WIND_KNOTS,
    keys::FORMAT_WIND_MPS,
    keys::FORMAT_PRESSURE_HPA,
    keys::FORMAT_PRESSURE_INHG,
    keys::FORMAT_PRESSURE_MMHG,
    keys::FORMAT_DISTANCE_KM,
    keys::FORMAT_DISTANCE_MI,
    keys::FORMAT_HUMIDITY,
    keys::FORMAT_UV,
    keys::FORMAT_PRECIP_MM,
    keys::FORMAT_PRECIP_IN,
    keys::DATE_ISO,
    keys::DATE_SHORT,
    keys::DATE_TODAY,
    keys::CONDITION_UNKNOWN,
    keys::PARTS[0],
    keys::PARTS[1],
    keys::PARTS[2],
    keys::PARTS[3],
    keys::WEEKDAYS[0],
    keys::WEEKDAYS[1],
    keys::WEEKDAYS[2],
    keys::WEEKDAYS[3],
    keys::WEEKDAYS[4],
    keys::WEEKDAYS[5],
    keys::WEEKDAYS[6],
    keys::MONTHS[0],
    keys::MONTHS[1],
    keys::MONTHS[2],
    keys::MONTHS[3],
    keys::MONTHS[4],
    keys::MONTHS[5],
    keys::MONTHS[6],
    keys::MONTHS[7],
    keys::MONTHS[8],
    keys::MONTHS[9],
    keys::MONTHS[10],
    keys::MONTHS[11],
    keys::UV_BANDS[0],
    keys::UV_BANDS[1],
    keys::UV_BANDS[2],
    keys::UV_BANDS[3],
    keys::UV_BANDS[4],
    keys::DIRECTIONS[0],
    keys::DIRECTIONS[1],
    keys::DIRECTIONS[2],
    keys::DIRECTIONS[3],
    keys::DIRECTIONS[4],
    keys::DIRECTIONS[5],
    keys::DIRECTIONS[6],
    keys::DIRECTIONS[7],
    keys::DIRECTIONS[8],
    keys::DIRECTIONS[9],
    keys::DIRECTIONS[10],
    keys::DIRECTIONS[11],
    keys::DIRECTIONS[12],
    keys::DIRECTIONS[13],
    keys::DIRECTIONS[14],
    keys::DIRECTIONS[15],
];

// ---------------------------------------------------------------------------------------------
// Language identity and negotiation
// ---------------------------------------------------------------------------------------------

/// A language a report can be rendered in.
///
/// The tag is the parsed, normalised spelling — `zh_CN.UTF-8` and `zh-cn` both become `zh-CN` — so
/// two spellings of one language cannot disagree about being the same language. The identifier is
/// re-parsed on demand: it is needed a handful of times per run (chain building, bundle
/// construction), never in a renderer's inner loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageId {
    /// The built-in language: the fallback of every chain and the only one that must exist.
    EnUs,
    /// Simplified Chinese.
    ZhCn,
}

impl LanguageId {
    /// The built-in language as a value, for callers that do not start from a request.
    pub const EN_US: Self = Self::EnUs;

    /// The BCP-47 tag, e.g. `en-US`.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    /// The parsed identifier, for chain building.
    #[must_use]
    pub fn identifier(self) -> LanguageIdentifier {
        self.tag().parse().expect("catalog tags are valid BCP-47")
    }

    /// The catalog language for `tag`, when the build ships one.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        CATALOGS
            .iter()
            .find_map(|(catalog, _)| (*catalog == tag).then(|| Self::from_catalog(catalog)))
    }

    /// The language of one `CATALOGS` entry.
    fn from_catalog(tag: &str) -> Self {
        match tag {
            "zh-CN" => Self::ZhCn,
            _ => Self::EnUs,
        }
    }
}

impl Default for LanguageId {
    fn default() -> Self {
        Self::EN_US
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.tag())
    }
}

/// What the user asked for, before the environment and the available catalogs are consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageRequest {
    /// `auto`, or no setting at all: negotiate from `LC_ALL`/`LC_MESSAGES`/`LANG`.
    Auto,
    /// An explicit tag, e.g. `zh-CN`, normalised from the value the user wrote.
    Tag(String),
}

impl LanguageRequest {
    /// Reads a `--lang` value (or a `defaults.language` setting).
    ///
    /// The `C`/`POSIX` pseudo-locales mean "no locale", which this client spells `auto`; every other
    /// value is normalised to a BCP-47 tag (`zh_CN.UTF-8` becomes `zh-CN`) and kept as an explicit
    /// request, whether or not it parses. An unparsable or unavailable tag is not an error — the
    /// caller warns and continues in `en-US`, because a typo in `--lang` must not cost a forecast.
    #[must_use]
    pub fn parse(request: &str) -> Self {
        let request = request.trim();
        if request.is_empty() || request.eq_ignore_ascii_case("auto") || is_pseudo_locale(request) {
            return Self::Auto;
        }
        // The same normalisation the ambient path uses, so `--lang zh_CN.UTF-8` is the tag
        // `zh-CN` before anything compares it against the catalogs. A value that normalises to
        // nothing (`!!`) is kept verbatim and rejected by the caller with a warning.
        let candidate = normalize_locale(request).unwrap_or_else(|| request.to_owned());
        if let Ok(identifier) = candidate.parse::<LanguageIdentifier>()
            && let Some(language) = LanguageId::from_tag(&identifier.to_string())
        {
            return Self::Tag(language.tag().to_owned());
        }
        Self::Tag(candidate)
    }

    /// The tag as the user wrote it, for a warning or a `-v` line; `auto` for the negotiated case.
    #[must_use]
    pub fn tag(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::Tag(tag) => tag,
        }
    }
}

/// Whether `value` is one of the two locales that mean "no locale at all".
fn is_pseudo_locale(value: &str) -> bool {
    value == "C" || value == "POSIX"
}

/// Normalises an ambient locale value (`zh_CN.UTF-8`) to a BCP-47 tag (`zh-CN`).
///
/// `None` for the values that carry no language: an unset or empty variable, and the `C`/`POSIX`
/// pseudo-locales. Everything after the first `.` (the codeset) and `@` (the modifier, as in
/// `sr_RS@latin`) is dropped, because a catalog is chosen by language, not by encoding.
fn normalize_locale(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || is_pseudo_locale(value) {
        return None;
    }
    let value = value
        .split(['.', '@'])
        .next()
        .unwrap_or(value)
        .replace('_', "-");
    let identifier: LanguageIdentifier = value.parse().ok()?;
    Some(identifier.to_string())
}

/// The fallback chain from `requested` down to [`DEFAULT_TAG`].
///
/// The map is explicit rather than algorithmic: a region-less `zh` and a Traditional `zh-TW` both
/// land on `zh-CN` (never on a generic `zh` match), everything else lands on `en-US`, and the
/// identifiers this build cannot answer with are kept in the chain so the `-v` line shows what was
/// skipped rather than silently shortening.
#[must_use]
pub fn language_chain(requested: &LanguageIdentifier) -> Vec<LanguageIdentifier> {
    let default: LanguageIdentifier = DEFAULT_TAG.parse().expect("en-US is a valid BCP-47 tag");
    let mut chain = vec![requested.clone()];
    if requested.language.as_str() == "zh" {
        let chinese: LanguageIdentifier = CHINESE_FALLBACK
            .parse()
            .expect("zh-CN is a valid BCP-47 tag");
        if *requested != chinese {
            chain.push(chinese);
        }
    }
    if chain.last() != Some(&default) {
        chain.push(default);
    }
    chain
}

// ---------------------------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------------------------

/// Something the run wants to tell the user about the language it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The requested tag has no catalog; the answer comes from a lower link of the chain.
    Fallback {
        /// The tag as it was requested.
        requested: String,
        /// The language actually rendered.
        selected: String,
    },
    /// The requested tag is not BCP-47 at all, so no chain could be built for it.
    UnknownTag(String),
    /// A catalog has no message for a key a renderer asked for (a bug in this crate).
    MissingKey(String),
}

impl Note {
    /// The note as one stderr line.
    ///
    /// A fallback and an unknown tag read the same way to the user — the request could not be
    /// honoured — while the difference stays in the type for the verbose report. Both name the
    /// value that was asked for and the catalogs this build actually ships, so the message answers
    /// "what can I write instead?" without a second command.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Fallback {
                requested,
                selected,
            } => {
                format!(
                    "warning: unsupported language \"{requested}\", falling back to {selected} \
                     (available: {})",
                    available_tags()
                )
            }
            Self::UnknownTag(requested) => format!(
                "warning: unsupported language \"{requested}\", falling back to {DEFAULT_TAG} \
                 (available: {})",
                available_tags()
            ),
            Self::MissingKey(key) => format!("warning: no message for key `{key}`"),
        }
    }
}

/// The catalog tags this build ships, for the "available" list of a language warning.
fn available_tags() -> String {
    CATALOGS
        .iter()
        .map(|(tag, _)| *tag)
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------------------------
// The catalog
// ---------------------------------------------------------------------------------------------

/// The message table of one run.
///
/// Built once, by [`I18n::load`], and borrowed by the renderers. The bundle owns its parsed
/// resources, so the common lookup returns a borrowed string rather than a copy.
pub struct I18n {
    language: LanguageId,
    /// The tag as it was requested, when it differs from the selected one: the `-v` line starts
    /// the chain there, so a substitution is shown from the reader's own spelling.
    requested: Option<String>,
    bundle: Option<FluentBundle<FluentResource>>,
    notes: RefCell<Vec<Note>>,
}

impl fmt::Debug for I18n {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("I18n")
            .field("language", &self.language)
            .field("requested", &self.requested)
            .field("catalogs", &self.bundle.is_some())
            .field("notes", &self.notes.borrow())
            .finish()
    }
}

impl I18n {
    /// Builds the catalog for `request`, reading the environment through the closure it is given.
    ///
    /// Infallible by design: an unparsable or unavailable tag is a note, never an error, because
    /// falling back to English still shows the user the weather. The closure is the seam that makes
    /// negotiation testable — the CLI passes `|name| std::env::var(name).ok()`, a test passes a map,
    /// and no test has to mutate the process environment.
    #[must_use]
    pub fn load(request: &LanguageRequest, environment: impl Fn(&str) -> Option<String>) -> Self {
        match request {
            LanguageRequest::Auto => {
                let (language, requested) = negotiate(&environment);
                let requested = requested.filter(|tag| tag != language.tag());
                Self::with_language(language, requested, Vec::new())
            }
            LanguageRequest::Tag(requested) => {
                let (language, notes) = resolve_tag(requested);
                let requested = (requested != language.tag()).then(|| requested.clone());
                Self::with_language(language, requested, notes)
            }
        }
    }

    /// Builds the catalog for a language: the chain's resources, in reverse priority.
    fn with_language(language: LanguageId, requested: Option<String>, notes: Vec<Note>) -> Self {
        Self {
            language,
            requested,
            bundle: build_bundle(language),
            notes: RefCell::new(notes),
        }
    }

    /// The language this run renders in.
    #[must_use]
    pub const fn lang(&self) -> LanguageId {
        self.language
    }

    /// The notes collected while resolving the language and while rendering.
    #[must_use]
    pub fn notes(&self) -> Vec<Note> {
        self.notes.borrow().clone()
    }

    /// The warning lines a run prints unless `-q` silences them.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.notes
            .borrow()
            .iter()
            .filter(|note| !matches!(note, Note::MissingKey(_)))
            .map(Note::text)
            .collect()
    }

    /// The one line `-v` prints about the language resolution: what was asked for, what was
    /// selected, and the chain between them.
    ///
    /// The requested spelling is the value the user wrote — `zh-TW`, or the ambient `zh_TW.UTF-8` —
    /// so a substitution (`zh-TW.UTF-8 → selected zh-CN`) is visible instead of looking like
    /// obedience, and a run that got exactly what it asked for says only which language that was.
    #[must_use]
    pub fn report(&self) -> String {
        let selected = self.language.tag();
        let start = self
            .requested
            .as_deref()
            .and_then(|raw| {
                raw.parse::<LanguageIdentifier>()
                    .ok()
                    .or_else(|| normalize_locale(raw)?.parse().ok())
            })
            .unwrap_or_else(|| self.language.identifier());
        let chain = format_chain(&start);
        match self.requested.as_deref() {
            Some(requested) => {
                format!("i18n: requested {requested} → selected {selected} (chain {chain})")
            }
            None => format!("i18n: {selected} (chain {chain})"),
        }
    }

    /// The message for `key`, or the key itself when no catalog has it.
    #[must_use]
    pub fn text(&self, key: &MessageKey) -> Cow<'_, str> {
        let name = key.as_str();
        if let Some(text) = self.lookup(&name) {
            return text;
        }
        self.missing(&name);
        Cow::Owned(name.into_owned())
    }

    /// The message for `key`, formatted with `arguments`.
    ///
    /// A pattern error counts as a missing key: the raw key is returned and the run is told once
    /// per key, so a mistyped argument in a catalog is visible in the output instead of printing
    /// `{$name}` in the middle of a table.
    #[must_use]
    pub fn format(
        &self,
        key: &MessageKey,
        arguments: &[(&str, FluentValue<'static>)],
    ) -> Cow<'_, str> {
        let name = key.as_str();
        let Some(bundle) = self.bundle.as_ref() else {
            self.missing(&name);
            return Cow::Owned(name.into_owned());
        };
        let Some(message) = bundle.get_message(&name) else {
            self.missing(&name);
            return Cow::Owned(name.into_owned());
        };
        let Some(pattern) = message.value() else {
            self.missing(&name);
            return Cow::Owned(name.into_owned());
        };
        let mut args = FluentArgs::new();
        for (argument, value) in arguments {
            args.set(*argument, value.clone());
        }
        let mut errors = Vec::new();
        let rendered = bundle.format_pattern(pattern, Some(&args), &mut errors);
        if !errors.is_empty() {
            self.missing(&name);
            return Cow::Owned(name.into_owned());
        }
        Cow::Owned(rendered.into_owned())
    }

    /// The name of a canonical condition.
    ///
    /// The selected catalog answers first, and a code this build cannot name falls back to the
    /// model's English description rather than to a raw key a reader cannot interpret.
    #[must_use]
    pub fn condition(&self, condition: Condition) -> Cow<'_, str> {
        self.text(&condition_key(condition))
    }

    /// The label of one day part.
    #[must_use]
    pub fn day_part(&self, part: DayPartKind) -> Cow<'_, str> {
        self.text(&day_part_key(part))
    }

    /// The short name of a weekday.
    #[must_use]
    pub fn weekday(&self, weekday: chrono::Weekday) -> Cow<'_, str> {
        self.text(&weekday_key(weekday))
    }

    /// The short name of a month.
    #[must_use]
    pub fn month(&self, month: chrono::Month) -> Cow<'_, str> {
        self.text(&month_key(month))
    }

    /// The band name of a UV index, as the WHO scale defines it.
    ///
    /// The bands are cut on the integer the token prints — `2.9` rounds to `3` and is `moderate`,
    /// exactly like `3.0` — so the number and its name can never disagree (`%u`/`%U` print `3` for
    /// `2.9`).
    #[must_use]
    pub fn uv_band(&self, uv: f32) -> Cow<'_, str> {
        self.text(&uv_band_key(uv))
    }

    /// The name of a compass point, e.g. `NE` or `东北风`.
    #[must_use]
    pub fn direction(&self, degrees: u16) -> Cow<'_, str> {
        self.text(&direction_key(degrees))
    }

    /// A temperature in the resolved unit, e.g. `+22°C` or `73°F`.
    ///
    /// `signed` is what the art table wants (`+22°C`); the prose-like formats leave it off. The
    /// number is formatted by [`crate::model::units`], whose formatters own the conversion and the
    /// rounding, and only the spelling of the unit comes from the catalog.
    #[must_use]
    pub fn format_temp(
        &self,
        celsius: f32,
        unit: crate::model::units::TempUnit,
        signed: bool,
    ) -> String {
        let value = if signed {
            crate::model::units::format_temp_signed(celsius, unit)
        } else {
            crate::model::units::format_temp(celsius, unit)
        };
        // The unit is already part of the formatter's output; the catalog message exists so a
        // translation could change the spacing or the symbol (`22 ℃`).
        let number = value
            .strip_suffix(unit.symbol())
            .unwrap_or(&value)
            .to_owned();
        let key = match unit {
            crate::model::units::TempUnit::Celsius => keys::FORMAT_TEMP_C,
            crate::model::units::TempUnit::Fahrenheit => keys::FORMAT_TEMP_F,
        };
        self.format(&key, &[("value", FluentValue::from(number))])
            .into_owned()
    }

    /// A date in the requested style.
    ///
    /// Every style is a Fluent message assembled from the catalog's own weekday and month names, so
    /// the order (`Wed 30 Sep`, `9月30日 周三`) is the translator's and never `chrono`'s English.
    #[must_use]
    pub fn format_date(&self, date: NaiveDate, style: DateStyle) -> String {
        let key = match style {
            DateStyle::Iso => keys::DATE_ISO,
            DateStyle::Short => keys::DATE_SHORT,
            DateStyle::Today => keys::DATE_TODAY,
        };
        let month = chrono::Month::try_from(u8::try_from(date.month()).unwrap_or(1))
            .unwrap_or(chrono::Month::January);
        // `day` and `month-number` are zero padded for the catalogs that write dates that way
        // (`2026-09-30`, `Thu 01 Oct`); `day-plain` is the bare number, for the ones that do not
        // (`9月30日`). Which one a language uses is the catalog's decision, not the caller's.
        self.format(
            &key,
            &[
                (
                    "weekday",
                    FluentValue::from(self.weekday(date.weekday()).into_owned()),
                ),
                ("month", FluentValue::from(self.month(month).into_owned())),
                (
                    "month-number",
                    FluentValue::from(format!("{:02}", date.month())),
                ),
                ("day", FluentValue::from(format!("{:02}", date.day()))),
                ("day-plain", FluentValue::from(date.day().to_string())),
                ("year", FluentValue::from(date.year().to_string())),
            ],
        )
        .into_owned()
    }

    /// The heading of a forecast day: `Today, Sep 30` for the location's own today, `Wed, Sep 30`
    /// for every other date.
    #[must_use]
    pub fn format_day_heading(&self, date: NaiveDate, today: NaiveDate) -> String {
        let style = if date == today {
            DateStyle::Today
        } else {
            DateStyle::Short
        };
        self.format_date(date, style)
    }

    /// The pattern for `key`, when a catalog has it.
    fn lookup(&self, key: &str) -> Option<Cow<'_, str>> {
        let bundle = self.bundle.as_ref()?;
        let message = bundle.get_message(key)?;
        let pattern = message.value()?;
        let mut errors = Vec::new();
        let rendered = bundle.format_pattern(pattern, None, &mut errors);
        errors.is_empty().then_some(rendered)
    }

    /// Records a missing key, at most once per key.
    ///
    /// Per *key*, not per run: a catalog that lacks two messages has to name both, otherwise the
    /// second one stays invisible until the first is fixed. The scan is over the handful of notes a
    /// run can collect, so it costs nothing next to rendering.
    fn missing(&self, key: &str) {
        let mut notes = self.notes.borrow_mut();
        if !notes
            .iter()
            .any(|note| matches!(note, Note::MissingKey(recorded) if recorded == key))
        {
            notes.push(Note::MissingKey(key.to_owned()));
        }
    }
}

/// The style [`I18n::format_date`] renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// The ISO order (`2026-09-30`), which stays language-independent in every catalog.
    Iso,
    /// A short date, `Wed, Sep 30` / `9月30日 周三`.
    Short,
    /// The short date with today in front, for the table's first column.
    Today,
}

// ---------------------------------------------------------------------------------------------
// Key lookups
// ---------------------------------------------------------------------------------------------

/// The key of a canonical condition, e.g. `cond-61`; undescribed codes share `cond-unknown`.
#[must_use]
pub fn condition_key(condition: Condition) -> MessageKey {
    if condition.is_known() {
        MessageKey(KeySource::Condition(condition.code()))
    } else {
        keys::CONDITION_UNKNOWN
    }
}

/// The key of a day part.
#[must_use]
pub fn day_part_key(part: DayPartKind) -> MessageKey {
    keys::PARTS[match part {
        DayPartKind::Morning => 0,
        DayPartKind::Noon => 1,
        DayPartKind::Evening => 2,
        DayPartKind::Night => 3,
    }]
}

/// The key of a weekday.
#[must_use]
pub fn weekday_key(weekday: chrono::Weekday) -> MessageKey {
    keys::WEEKDAYS[weekday.num_days_from_monday() as usize]
}

/// The key of a month.
#[must_use]
pub fn month_key(month: chrono::Month) -> MessageKey {
    keys::MONTHS[month.number_from_month() as usize - 1]
}

/// The key of a UV band.
///
/// The bands are cut on the value `%U` prints — [`crate::model::units::fmt_int`]'s rounded integer
/// — so the number and its name can never disagree: `2.9` rounds to `3` and both are `moderate`.
/// Cutting on the raw `f32` instead would print `3 (low)` at `2.9` and `3 (moderate)` at `3.0`.
#[must_use]
pub fn uv_band_key(uv: f32) -> MessageKey {
    let rounded = crate::model::units::round_half_away_from_zero(uv);
    let index = if rounded < 3.0 {
        0
    } else if rounded < 6.0 {
        1
    } else if rounded < 8.0 {
        2
    } else if rounded < 11.0 {
        3
    } else {
        4
    };
    keys::UV_BANDS[index]
}

/// The key of a compass point, given the wind direction's degree reading.
///
/// The sixteen points are the ones [`crate::model::units::compass_16`] names, so the catalog's
/// direction and the renderers' arrows cannot drift apart.
#[must_use]
pub fn direction_key(degrees: u16) -> MessageKey {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let point = crate::model::units::compass_16(degrees);
    let index = POINTS
        .iter()
        .position(|candidate| *candidate == point)
        .unwrap_or(0);
    keys::DIRECTIONS[index]
}

// ---------------------------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------------------------

/// Parses one catalog source. The error is a plain string: this runs before a renderer exists.
fn resource(source: &str) -> std::result::Result<FluentResource, String> {
    FluentResource::try_new(source.to_owned()).map_err(|(_, errors)| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// The bundle for `language`: the chain's resources, added so the selected language wins.
///
/// [`FluentBundle::add_resource_overriding`] makes a later resource replace an earlier definition of
/// the same id, so walking the chain backwards — `en-US` first, the selected language last — leaves
/// the requested language in front and English behind every key it does not define. That ordering
/// is the fallback contract, and the test on this function pins it. A build whose catalogs do not
/// parse gets `None`: every key then resolves to its own name, which is honest output rather than a
/// panic on a path the catalog test already forbids.
fn build_bundle(language: LanguageId) -> Option<FluentBundle<FluentResource>> {
    let mut bundle = FluentBundle::new(vec![language.identifier()]);
    bundle.set_use_isolating(false);
    for identifier in language_chain(&language.identifier()).iter().rev() {
        let tag = identifier.to_string();
        let Some((_, source)) = CATALOGS
            .iter()
            .copied()
            .find(|(catalog, _)| *catalog == tag)
        else {
            continue;
        };
        let resource = resource(source).ok()?;
        bundle.add_resource_overriding(resource);
    }
    Some(bundle)
}

/// The catalog language a parsed tag resolves to, if this build can serve it at all.
///
/// Two cases: the tag is a catalog, or it belongs to a family whose fallback map is explicit —
/// `zh-*` lands on `zh-CN`, `en-*` on `en-US`, and both are the language [`language_chain`] names,
/// so no warning is owed for a substitution the chain documents. Everything else — `de-DE`,
/// `fr-CA` — has no answer, and the caller decides between warning (an explicit request) and
/// walking on (an ambient locale).
fn catalog_for(identifier: &LanguageIdentifier) -> Option<LanguageId> {
    if let Some(language) = LanguageId::from_tag(&identifier.to_string()) {
        return Some(language);
    }
    match identifier.language.as_str() {
        "zh" => Some(LanguageId::ZhCn),
        "en" => Some(LanguageId::EnUs),
        _ => None,
    }
}

/// The language an explicit request resolves to, with the notes that explain the choice.
///
/// A request the build cannot serve at all (`de-DE`, `bad-TAG`) is a fallback to English and warns.
/// An `en`/`zh` family request resolves through the documented chain — `en-GB → en-US`,
/// `zh-TW → zh-CN` — without a warning: the `-v` line prints the substitution, and warning about a
/// substitution the plan promises would be noise.
fn resolve_tag(requested: &str) -> (LanguageId, Vec<Note>) {
    let Ok(identifier) = requested.parse::<LanguageIdentifier>() else {
        return (
            LanguageId::EN_US,
            vec![Note::UnknownTag(requested.to_owned())],
        );
    };
    let Some(language) = catalog_for(&identifier) else {
        return (
            LanguageId::EN_US,
            vec![Note::Fallback {
                requested: requested.to_owned(),
                selected: LanguageId::EN_US.tag().to_owned(),
            }],
        );
    };
    (language, Vec::new())
}

/// The first ambient locale that names a language this build has a catalog for, and the value it
/// was spelled as.
///
/// The order is the POSIX one: `LC_ALL` beats `LC_MESSAGES` beats `LANG`. A variable that names a
/// language without a catalog does not stop the walk — `LANG=de_DE.UTF-8` still leaves
/// `LC_MESSAGES` free to name `zh_CN` — and an empty value counts as unset, which is what the
/// `LC_ALL=` idiom means.
fn negotiate(environment: &impl Fn(&str) -> Option<String>) -> (LanguageId, Option<String>) {
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Some(value) = environment(name) else {
            continue;
        };
        let Some(tag) = normalize_locale(&value) else {
            continue;
        };
        let Ok(identifier) = tag.parse::<LanguageIdentifier>() else {
            continue;
        };
        if let Some(language) = catalog_for(&identifier) {
            return (language, Some(value.trim().to_owned()));
        }
    }
    (LanguageId::EN_US, None)
}

/// The chain, formatted for the `-v` line.
fn format_chain(requested: &LanguageIdentifier) -> String {
    language_chain(requested)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" → ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        CATALOGS, DEFAULT_TAG, DateStyle, I18n, LanguageId, LanguageRequest, MessageKey, Note,
        RENDERER_KEYS, condition_key, day_part_key, direction_key, keys, month_key, uv_band_key,
        weekday_key,
    };
    use crate::model::DayPartKind;
    use crate::model::condition::Condition;
    use crate::model::units::TempUnit;

    /// An environment lookup over a fixed map — the seam [`I18n::load`] takes.
    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// The English catalog, loaded the way the CLI loads it when nothing is configured.
    pub(super) fn english() -> I18n {
        I18n::load(&LanguageRequest::Auto, environment(&[]))
    }

    /// A catalog for one tag, for the unit tests of the other modules.
    pub(super) fn catalog(tag: &str) -> I18n {
        if tag == DEFAULT_TAG {
            return english();
        }
        I18n::load(&LanguageRequest::Tag(tag.to_owned()), environment(&[]))
    }

    #[test]
    fn the_built_in_language_is_the_default() {
        assert_eq!(DEFAULT_TAG, "en-US");
        assert_eq!(LanguageId::EN_US.tag(), "en-US");
        assert_eq!(LanguageId::default(), LanguageId::EN_US);
        assert_eq!(LanguageId::EN_US.to_string(), "en-US");
    }

    #[test]
    fn negotiation_reads_the_ambient_chain() {
        for (pairs, expected) in [
            (vec![], "en-US"),
            (vec![("LANG", "zh_CN.UTF-8")], "zh-CN"),
            (vec![("LC_MESSAGES", "zh_TW.UTF-8")], "zh-CN"),
            (
                vec![("LC_ALL", "de_DE.UTF-8"), ("LANG", "zh_CN.UTF-8")],
                "zh-CN",
            ),
            (vec![("LC_ALL", "C"), ("LANG", "zh_CN.UTF-8")], "zh-CN"),
            (
                vec![("LANG", "de_DE.UTF-8"), ("LC_MESSAGES", "zh_CN")],
                "zh-CN",
            ),
            (vec![("LANG", "POSIX")], "en-US"),
            (vec![("LANG", "")], "en-US"),
            (vec![("LANG", "de_DE.UTF-8")], "en-US"),
            (vec![("LANG", "en_GB.UTF-8")], "en-US"),
            (vec![("LANG", "zh_CN.UTF-8@latin")], "zh-CN"),
        ] {
            let i18n = I18n::load(&LanguageRequest::Auto, environment(&pairs));
            assert_eq!(i18n.lang().tag(), expected, "{pairs:?}");
            assert_eq!(i18n.warnings(), Vec::<String>::new(), "{pairs:?}");
            assert!(i18n.report().starts_with("i18n:"), "{pairs:?}");
        }
    }

    #[test]
    fn an_explicit_request_wins_and_reports_its_fallbacks() {
        for (requested, expected, warned) in [
            ("zh-CN", "zh-CN", false),
            ("zh-TW", "zh-CN", false),
            ("zh-HK", "zh-CN", false),
            ("en", "en-US", false),
            ("en-GB", "en-US", false),
            ("EN", "en-US", false),
            ("en-US", "en-US", false),
            ("de-DE", "en-US", true),
            ("bad-TAG", "en-US", true),
            ("!!", "en-US", true),
        ] {
            let request = LanguageRequest::Tag(requested.to_owned());
            // The ambient locale must not influence an explicit request.
            let i18n = I18n::load(&request, environment(&[("LANG", "zh_CN.UTF-8")]));
            assert_eq!(i18n.lang().tag(), expected, "{requested}");
            let warnings = i18n.warnings();
            assert_eq!(!warnings.is_empty(), warned, "{requested}: {warnings:?}");
            if warned {
                assert!(
                    warnings.iter().any(|warning| warning.contains(requested)),
                    "{requested}: {warnings:?}"
                );
            }
            let _ = expected;
        }
    }

    #[test]
    fn a_fallback_never_renders_the_language_that_was_not_asked_for() {
        // The Traditional request is served by the Simplified catalog, and `-v` is where the
        // substitution is disclosed: `zh-TW → zh-CN → en-US`.
        let request = LanguageRequest::Tag("zh-TW".to_owned());
        let i18n = I18n::load(&request, environment(&[]));
        assert_eq!(i18n.lang().tag(), "zh-CN");
        assert_eq!(i18n.text(&keys::LABEL_REPORT), "天气报告：");
        let report = i18n.report();
        assert!(report.contains("requested zh-TW"), "{report}");
        assert!(report.contains("selected zh-CN"), "{report}");
        assert!(report.contains("zh-TW → zh-CN → en-US"), "{report}");
    }

    #[test]
    fn the_keys_name_their_conditions_and_calendars() {
        assert_eq!(condition_key(Condition::from_u8(61)).as_str(), "cond-61");
        assert_eq!(condition_key(Condition::from_u8(95)).as_str(), "cond-95");
        for code in [8_u8, 20, 46, 100, 255] {
            assert_eq!(
                condition_key(Condition::from_u8(code)).as_str(),
                "cond-unknown",
                "{code}"
            );
        }
        assert_eq!(day_part_key(DayPartKind::Morning).as_str(), "part-morning");
        assert_eq!(day_part_key(DayPartKind::Night).as_str(), "part-night");
        assert_eq!(weekday_key(chrono::Weekday::Wed).as_str(), "weekday-wed");
        assert_eq!(month_key(chrono::Month::September).as_str(), "month-9");
        assert_eq!(month_key(chrono::Month::December).as_str(), "month-12");
        for (uv, key) in [
            (0.0, "uv-band-low"),
            (2.4, "uv-band-low"),
            (2.5, "uv-band-moderate"),
            (2.9, "uv-band-moderate"),
            (3.0, "uv-band-moderate"),
            (5.4, "uv-band-moderate"),
            (5.5, "uv-band-high"),
            (5.9, "uv-band-high"),
            (6.0, "uv-band-high"),
            (7.4, "uv-band-high"),
            (7.5, "uv-band-very-high"),
            (7.9, "uv-band-very-high"),
            (8.0, "uv-band-very-high"),
            (10.4, "uv-band-very-high"),
            (10.9, "uv-band-extreme"),
            (11.0, "uv-band-extreme"),
        ] {
            assert_eq!(uv_band_key(uv).as_str(), key, "UV {uv}");
        }
        assert_eq!(direction_key(0).as_str(), "dir-n");
        assert_eq!(direction_key(45).as_str(), "dir-ne");
        assert_eq!(direction_key(90).as_str(), "dir-e");
        assert_eq!(direction_key(225).as_str(), "dir-sw");
        assert_eq!(direction_key(360).as_str(), "dir-n");
    }

    #[test]
    fn every_catalog_parses_and_carries_the_same_keys() {
        let english = key_set(CATALOGS[0].1);
        assert!(
            english.contains("cond-0") && english.contains("cond-99"),
            "the English catalog spells out every code"
        );
        for (tag, source) in CATALOGS {
            let catalog = key_set(source);
            let missing: Vec<&String> = english.difference(&catalog).collect();
            let orphan: Vec<&String> = catalog.difference(&english).collect();
            assert_eq!(missing, Vec::<&String>::new(), "{tag} lacks");
            assert_eq!(
                orphan,
                Vec::<&String>::new(),
                "{tag} has keys en-US does not:"
            );
        }
    }

    /// The message ids of one catalog source, as the FTL syntax defines them: a key at the start of
    /// a line, then `=`. Enough for the flat catalogs this crate ships, and independent of the
    /// renderers the completeness test guards.
    fn key_set(source: &str) -> BTreeSet<String> {
        source
            .lines()
            .filter(|line| !line.starts_with([' ', '#', '\t']) && !line.trim().is_empty())
            .filter_map(|line| line.split_once('=').map(|(key, _)| key.trim().to_owned()))
            .collect()
    }

    #[test]
    fn the_english_catalog_renders_its_messages() {
        let i18n = english();
        assert_eq!(i18n.text(&keys::LABEL_REPORT), "Weather report:");
        assert_eq!(i18n.text(&keys::LABEL_LOCATION), "location");
        assert_eq!(i18n.condition(Condition::from_u8(61)), "Slight rain");
        assert_eq!(i18n.condition(Condition::from_u8(95)), "Thunderstorm");
        assert_eq!(i18n.condition(Condition::from_u8(10)), "Mist");
        assert_eq!(i18n.condition(Condition::from_u8(8)), "Unknown");
        assert_eq!(i18n.day_part(DayPartKind::Morning), "Morning");
        assert_eq!(i18n.weekday(chrono::Weekday::Wed), "Wed");
        assert_eq!(i18n.month(chrono::Month::September), "Sep");
        assert_eq!(i18n.uv_band(5.0), "moderate");
        assert_eq!(i18n.direction(45), "NE");
        assert_eq!(i18n.uv_band(12.0), "extreme");
        assert_eq!(i18n.text(&keys::NA), "n/a");
        assert_eq!(i18n.notes(), Vec::<Note>::new());
    }

    #[test]
    fn the_chinese_catalog_renders_its_messages() {
        let i18n = catalog("zh-CN");
        assert_eq!(i18n.text(&keys::LABEL_REPORT), "天气报告：");
        assert_eq!(i18n.condition(Condition::from_u8(61)), "小雨");
        assert_eq!(i18n.condition(Condition::from_u8(10)), "轻雾");
        assert_eq!(i18n.condition(Condition::from_u8(8)), "未知");
        assert_eq!(i18n.day_part(DayPartKind::Night), "夜间");
        assert_eq!(i18n.weekday(chrono::Weekday::Sun), "周日");
        assert_eq!(i18n.month(chrono::Month::January), "1月");
        assert_eq!(i18n.uv_band(7.0), "强");
        assert_eq!(i18n.notes(), Vec::<Note>::new());
    }

    #[test]
    fn a_missing_key_stays_visible_and_is_reported_once_per_key() {
        let i18n = english();
        let first = MessageKey::new("no-such-key");
        let second = MessageKey::new("another-missing-key");
        assert_eq!(i18n.text(&first), "no-such-key");
        assert_eq!(i18n.text(&second), "another-missing-key");
        // Repeating either key must not add a second note for it.
        assert_eq!(i18n.text(&first), "no-such-key");
        assert_eq!(i18n.text(&second), "another-missing-key");
        let reported: Vec<String> = i18n
            .notes()
            .iter()
            .filter_map(|note| match note {
                Note::MissingKey(key) => Some(key.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            reported,
            ["no-such-key".to_owned(), "another-missing-key".to_owned()],
            "every distinct missing key is named once: {:?}",
            i18n.notes()
        );
    }

    #[test]
    fn a_pattern_error_returns_the_key_instead_of_a_half_render() {
        // `format-temp-c` needs `{$value}`; a call site that forgets it must not leak `{$value}`
        // into a table — the raw key is what the doc promises and what `-v` then reports.
        let i18n = english();
        assert_eq!(
            i18n.format(&keys::FORMAT_TEMP_C, &[]),
            "format-temp-c",
            "{:?}",
            i18n.notes()
        );
        let missing = i18n
            .notes()
            .iter()
            .filter(|note| matches!(note, Note::MissingKey(_)))
            .count();
        assert_eq!(missing, 1, "{:?}", i18n.notes());
    }

    #[test]
    fn a_language_request_is_normalised_before_it_is_parsed() {
        // `--lang` and the ambient locale share one normalisation: a POSIX spelling must not be
        // rejected by the tier that outranks the environment.
        for (requested, expected) in [
            ("zh_CN.UTF-8", Some("zh-CN")),
            ("zh-cn", Some("zh-CN")),
            ("en_GB.UTF-8", Some("en-GB")),
            ("de_DE.UTF-8", Some("de-DE")),
            ("sr_RS@latin", Some("sr-RS")),
            ("C", None),
            ("POSIX", None),
            ("auto", None),
        ] {
            let request = LanguageRequest::parse(requested);
            match expected {
                Some(tag) => assert_eq!(request.tag(), tag, "{requested}"),
                None => assert_eq!(request, LanguageRequest::Auto, "{requested}"),
            }
        }
    }

    #[test]
    fn temperatures_and_dates_are_assembled_from_the_catalog() {
        let i18n = english();
        assert_eq!(i18n.format_temp(22.0, TempUnit::Celsius, true), "+22°C");
        assert_eq!(i18n.format_temp(22.0, TempUnit::Celsius, false), "22°C");
        assert_eq!(i18n.format_temp(22.0, TempUnit::Fahrenheit, false), "72°F");
        assert_eq!(i18n.format_temp(-5.2, TempUnit::Celsius, true), "-5°C");

        let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("a valid date");
        assert_eq!(i18n.format_date(date, DateStyle::Short), "Wed 30 Sep");
        assert_eq!(i18n.format_date(date, DateStyle::Iso), "2026-09-30");
        assert_eq!(i18n.format_date(date, DateStyle::Today), "Today, Sep 30");
        let first = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).expect("a valid date");
        assert_eq!(i18n.format_date(first, DateStyle::Short), "Thu 01 Oct");

        let chinese = catalog("zh-CN");
        assert_eq!(chinese.format_date(date, DateStyle::Short), "9月30日 周三");
        assert_eq!(chinese.format_date(date, DateStyle::Today), "今天 9月30日");
    }

    #[test]
    fn the_selected_language_is_scanned_before_the_fallback() {
        // The bundle adds the chain in reverse, so this is the ordering contract: a key both
        // catalogs define comes from the selected one, and a key only English defines still
        // resolves.
        let chinese = catalog("zh-CN");
        assert_eq!(chinese.text(&keys::PARTS[0]), "早上");
        assert_eq!(chinese.text(&keys::LABEL_DATA), "数据：");
    }

    #[test]
    fn formatting_never_emits_bidi_isolation_marks() {
        let i18n = catalog("zh-CN");
        let mut rendered = String::new();
        for key in RENDERER_KEYS {
            rendered.push_str(&i18n.text(key));
        }
        rendered.push_str(&i18n.format_temp(22.0, TempUnit::Celsius, true));
        rendered.push_str(&i18n.condition(Condition::from_u8(45)));
        assert!(!rendered.contains('\u{2068}'), "no FSI in {rendered}");
        assert!(!rendered.contains('\u{2069}'), "no PDI in {rendered}");
    }
}
