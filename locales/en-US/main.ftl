# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The en-US catalog: every user-visible word a renderer can ask for, and the fallback resource of
# every other language. `src/i18n.rs` embeds this file with `include_str!`; nothing reads it from
# disk at runtime.
#
# Conventions, enforced by `tests/i18n.rs`:
#   * message ids are lower case, hyphenated, and never contain an underscore (Fluent reads it as
#     subtraction); the 100 condition keys use `cond.<code>` plus `cond.unknown`;
#   * every key exists in every catalog — a missing translation is a build failure, not a silent
#     English string in the middle of Chinese output;
#   * a `format-*` message turns one already-converted value into its display string (the unit
#     table is `en-US` by design: weather is written in metric symbols in every shipped locale);
#   * a `label-*` message is a word on its own — the art table prints it as a heading and the
#     `plain` format uses it as the record key before the colon, where lower case is part of the
#     format contract (step 08), not a styling accident.

# --- Conditions: WMO 4677 ---------------------------------------------------------------
# The codes the model describes carry their own name; every other code in 0..=99 is unknown to
# this client and must still be nameable, which is why the whole range is spelled out.

cond-0 = Clear sky
cond-1 = Mainly clear
cond-2 = Partly cloudy
cond-3 = Overcast
cond-4 = Smoke
cond-5 = Haze
cond-6 = Widespread dust
cond-7 = Dust or sand raised by wind
cond-8 = Unknown
cond-9 = Unknown
cond-10 = Mist
cond-11 = Unknown
cond-12 = Unknown
cond-13 = Unknown
cond-14 = Unknown
cond-15 = Unknown
cond-16 = Unknown
cond-17 = Unknown
cond-18 = Unknown
cond-19 = Unknown
cond-20 = Unknown
cond-21 = Unknown
cond-22 = Unknown
cond-23 = Unknown
cond-24 = Unknown
cond-25 = Unknown
cond-26 = Unknown
cond-27 = Unknown
cond-28 = Unknown
cond-29 = Unknown
cond-30 = Unknown
cond-31 = Unknown
cond-32 = Unknown
cond-33 = Unknown
cond-34 = Unknown
cond-35 = Unknown
cond-36 = Unknown
cond-37 = Unknown
cond-38 = Unknown
cond-39 = Unknown
cond-40 = Unknown
cond-41 = Unknown
cond-42 = Unknown
cond-43 = Unknown
cond-44 = Unknown
cond-45 = Fog
cond-46 = Unknown
cond-47 = Unknown
cond-48 = Depositing rime fog
cond-49 = Unknown
cond-50 = Unknown
cond-51 = Light drizzle
cond-52 = Unknown
cond-53 = Moderate drizzle
cond-54 = Unknown
cond-55 = Dense drizzle
cond-56 = Light freezing drizzle
cond-57 = Dense freezing drizzle
cond-58 = Unknown
cond-59 = Unknown
cond-60 = Unknown
cond-61 = Slight rain
cond-62 = Unknown
cond-63 = Moderate rain
cond-64 = Unknown
cond-65 = Heavy rain
cond-66 = Light freezing rain
cond-67 = Heavy freezing rain
cond-68 = Unknown
cond-69 = Unknown
cond-70 = Unknown
cond-71 = Slight snow fall
cond-72 = Unknown
cond-73 = Moderate snow fall
cond-74 = Unknown
cond-75 = Heavy snow fall
cond-76 = Unknown
cond-77 = Snow grains
cond-78 = Unknown
cond-79 = Ice pellets
cond-80 = Slight rain showers
cond-81 = Moderate rain showers
cond-82 = Violent rain showers
cond-83 = Unknown
cond-84 = Unknown
cond-85 = Slight snow showers
cond-86 = Heavy snow showers
cond-87 = Unknown
cond-88 = Unknown
cond-89 = Unknown
cond-90 = Unknown
cond-91 = Unknown
cond-92 = Unknown
cond-93 = Unknown
cond-94 = Unknown
cond-95 = Thunderstorm
cond-96 = Thunderstorm with slight hail
cond-97 = Heavy thunderstorm
cond-98 = Unknown
cond-99 = Thunderstorm with heavy hail
cond-unknown = Unknown

# --- Day parts --------------------------------------------------------------------------

part-morning = Morning
part-noon = Noon
part-evening = Evening
part-night = Night

# --- Calendar names ---------------------------------------------------------------------
# Only for display: the model's own month/weekday numbers are what a provider is decoded with.

weekday-mon = Mon
weekday-tue = Tue
weekday-wed = Wed
weekday-thu = Thu
weekday-fri = Fri
weekday-sat = Sat
weekday-sun = Sun

month-1 = Jan
month-2 = Feb
month-3 = Mar
month-4 = Apr
month-5 = May
month-6 = Jun
month-7 = Jul
month-8 = Aug
month-9 = Sep
month-10 = Oct
month-11 = Nov
month-12 = Dec

# `$day` and `$month-number` are zero padded (`01`, `09`), `$day-plain` is the bare number (`1`);
# `$month` and `$weekday` are the names above. A language uses the spelling it writes.
date-iso = { $year }-{ $month-number }-{ $day }
date-short = { $weekday } { $day } { $month }
date-today = Today, { $month } { $day }

# --- Report and record labels -----------------------------------------------------------

label-report = Weather report:
label-data = Data:

# The line an observation-only report prints under its current block, and the footer that says why
# no day table follows.
label-observed = observed
format-age-minutes = { $minutes } min ago
format-age-hours = { $hours } h ago
note-no-forecast = no forecast: { $provider } is an observation

# The `plain` record keys. Lower case is the format: `location:` is greppable, and step 08 fixed
# it. A translation may change the word but not the shape of the line.
label-location = location
label-updated = updated
label-current = current
label-day = day
label-attribution = attribution

# --- Measurements -----------------------------------------------------------------------

label-feels = feels
label-wind = wind
label-humidity = humidity
label-precip = precip
label-pressure = pressure
label-visibility = visibility
label-uv = UV
label-sunrise = sunrise
label-sunset = sunset

# --- Values the renderers name ------------------------------------------------------------

uv-band-low = low
uv-band-moderate = moderate
uv-band-high = high
uv-band-very-high = very high
uv-band-extreme = extreme

na = n/a

# The compass rose: the sixteen points a wind direction can name. The arrow is deliberately not
# here — it is charset-dependent (a dumb terminal gets `^` where a UTF-8 one gets `^`'s arrow), which
# is a property of the terminal, not of the language. The renderer composes `arrow speed name`, and
# the catalog decides what the name reads: `NE`, `东北风`.
dir-n = N
dir-nne = NNE
dir-ne = NE
dir-ene = ENE
dir-e = E
dir-ese = ESE
dir-se = SE
dir-sse = SSE
dir-s = S
dir-ssw = SSW
dir-sw = SW
dir-wsw = WSW
dir-w = W
dir-wnw = WNW
dir-nw = NW
dir-nnw = NNW

# --- Formatting: one converted value into its display string ------------------------------

format-temp-c = { $value }°C
format-temp-f = { $value }°F
format-wind-kmh = { $value }km/h
format-wind-mph = { $value }mph
format-wind-knots = { $value }kn
format-wind-mps = { $value }m/s
format-pressure-hpa = { $value }hPa
format-pressure-inhg = { $value }inHg
format-pressure-mmhg = { $value }mmHg
format-distance-km = { $value }km
format-distance-mi = { $value }mi
format-humidity = { $value }%
format-uv = { $value } ({ $band })
format-precip-mm = { $value }mm
format-precip-in = { $value }in

# --- Alerts (step 15) ------------------------------------------------------------------------
# The CAP severity triple, the source names and the banner fragments. `alert-banner-line` is the
# first line of every alert in every format; the glyph and the colour are added by the renderer,
# so a translation controls the words and the punctuation only. The WMO SWIC and FPAS credits are
# the lines their terms require next to the warnings.

label-alert = alert

alert-none = no active weather alerts
alert-banner-line = { $event } — { $severity }
alert-until = until { $time }
alert-since = since { $time }
alert-more-count = … and { $count } more

alert-severity-unknown = Unknown
alert-severity-minor = Minor
alert-severity-moderate = Moderate
alert-severity-severe = Severe
alert-severity-extreme = Extreme

alert-urgency-unknown = Unknown
alert-urgency-past = Past
alert-urgency-future = Future
alert-urgency-expected = Expected
alert-urgency-immediate = Immediate

alert-certainty-unknown = Unknown
alert-certainty-unobserved = Unobserved
alert-certainty-possible = Possible
alert-certainty-unlikely = Unlikely
alert-certainty-likely = Likely
alert-certainty-observed = Observed

alert-source-nws = US National Weather Service
alert-source-meteoalarm = MeteoAlarm
alert-source-qweather = QWeather
alert-source-hko = Hong Kong Observatory
alert-source-wmoswic = WMO Severe Weather Information Centre
alert-source-fpas = FOSS Public Alert Server
alert-source-visualcrossing = Visual Crossing

alert-credit-wmoswic = Warnings by the WMO Severe Weather Information Centre (severeweather.wmo.int), © the issuing agencies
alert-credit-fpas = Warnings via the FOSS Public Alert Server ({ $host })

# --- Air quality (step 16) ---------------------------------------------------------------------
# The panel's labels, the two AQI category scales and the pollutant/pollen names. The category
# names are keyed by `AqiCategory::index`; `aqi-no-coverage` is the whole answer for a point the
# pollen model does not cover, and `aqi-uv-source` marks the panel's UV reading as weather data
# rather than an air measurement.

aqi-panel-title = Air quality
aqi-us-label = US AQI
aqi-european-label = European AQI
aqi-pollen-title = Pollen
aqi-no-coverage = not covered at this location
aqi-unavailable = air quality unavailable
aqi-uv-label = UV
aqi-uv-source = weather data

aqi-category-good = Good
aqi-category-moderate = Moderate
aqi-category-unhealthy-sensitive = Unhealthy for sensitive groups
aqi-category-unhealthy = Unhealthy
aqi-category-very-unhealthy = Very unhealthy
aqi-category-hazardous = Hazardous
aqi-category-fair = Fair
aqi-category-poor = Poor
aqi-category-very-poor = Very poor
aqi-category-extremely-poor = Extremely poor

pm2-5 = PM2.5
pm10 = PM10
o3 = O3
no2 = NO2
so2 = SO2
co = CO

pollen-alder = alder
pollen-birch = birch
pollen-grass = grass
pollen-mugwort = mugwort
pollen-olive = olive
pollen-ragweed = ragweed

unit-ug-m3 = μg/m³
unit-grains-m3 = grains/m³

# --- Moon and sun (step 17) --------------------------------------------------------------------
# The phase names are keyed by `MoonPhase::index`; the rise/set labels head the values in every
# format, and `astro-no-rise` is the placeholder for an event that does not happen on the day
# (the Moon's calendar day can miss one). The polar labels replace the whole sun line inside the
# circles, so no clock time is invented. `astro-computed` is the standalone view's provenance
# line, spoken only there.

label-moon = Moon
label-sun = Sun

moon-phase-new = New Moon
moon-phase-waxing-crescent = Waxing Crescent
moon-phase-first-quarter = First Quarter
moon-phase-waxing-gibbous = Waxing Gibbous
moon-phase-full = Full Moon
moon-phase-waning-gibbous = Waning Gibbous
moon-phase-last-quarter = Last Quarter
moon-phase-waning-crescent = Waning Crescent

astro-moonrise = Moonrise
astro-moonset = Moonset
astro-sunrise = Sunrise
astro-sunset = Sunset
astro-daylight = daylight
astro-polar-day = polar day
astro-polar-night = polar night
astro-no-rise = —
astro-illumination = { $percent }% illuminated (geocentric)
astro-age-days = age { $days } d
astro-next = Next phases
astro-computed = computed locally (no network)
