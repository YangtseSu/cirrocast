// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The offline country layer: Natural Earth's 1:50m admin-0 shapes, quantised (step 25).
//!
//! Step 18's city table answers *where* a coordinate is only when a city is near it; this layer
//! answers *which country* it is in, for any coordinate on Earth and without a socket. It is what
//! lets `@lat,lon` be named ("Xianghe, Hebei, China") and what gives an IP answer a country when
//! the IP service omits one.
//!
//! The data is Natural Earth's `ne_50m_admin_0_countries` (public domain, CC0-1.0), reduced to what
//! the lookup needs: the ISO 3166-1 alpha-2 code, the English name, and the polygons quantised to
//! 1e-3 degrees (~110 m). The committed member is 408 KiB gzipped against the 1 MiB budget the
//! step file names, which is why the 1:50m data ships instead of falling back to 1:10m's coarse
//! sibling; [`MAX_MEMBER_BYTES`] makes a future refresh that blows the budget a build failure
//! instead of a silent size regression.
//!
//! Three rules are worth restating because they are easy to undo by accident:
//!
//! * **the code is always usable or absent.** Natural Earth leaves `ISO_A2` as `-99` for the
//!   shapes that have no ISO code and for a few rows its corrected `ISO_A2_EH` field fixes
//!   (France, Norway, the Indian Ocean Territories). The parser prefers `ISO_A2_EH`, falls back to
//!   `ISO_A2`, then to [`DISPUTED_CODES`] — the four sovereignty-ambiguous shapes the step file
//!   names — and finally leaves the code *empty* rather than writing `-99` into a
//!   [`Location`](crate::model::Location), because `auto` provider selection and alert coverage
//!   read that field;
//! * **the geometry keeps `GeoJSON`'s ring order.** The first ring of a polygon is its outline and
//!   the rest are holes (Lesotho inside South Africa, the Vatican inside Italy), so the
//!   point-in-polygon test is "inside the outline and outside every hole" — a plain even-odd test
//!   over all rings would report a coordinate in Lesotho as South African;
//! * **the first match wins and the lookup is cheap.** The shapes do not overlap in Natural Earth,
//!   and a lookup walks the points of the countries in file order (99 613 points for the whole
//!   world, well under a millisecond), so no spatial index is worth its complexity here.
//!
//! The format is deliberately the city table's style — a magic, a count and length-prefixed
//! fields, little-endian throughout — and it reuses the city table's byte reader, gzip helper and
//! cap. Both members are written by `build/geo-table` (step 25's `--countries` mode).

use serde_json::Value;

use crate::error::{Error, Result};

/// The canonical source the committed layer is built from: the tagged Natural Earth release, so a
/// rebuild of the same tag is byte-identical.
pub const OFFICIAL_URL: &str = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_50m_admin_0_countries.geojson";

/// The gzipped member's name under `src/geo/data/`, and under a user's directory.
pub const MEMBER: &str = "countries.bin.gz";

/// The provenance record written beside the member.
pub const RECORD: &str = "COUNTRIES";

/// Format version of the member.
pub(crate) const COUNTRIES_MAGIC: &[u8; 5] = b"CCNE\x01";

/// The quantisation of the stored coordinates: 1e-3 degrees, about 110 m.
pub(crate) const SCALE: f64 = 1000.0;

/// The compressed size a member may not exceed: the step file's fallback budget, above which the
/// 1:10m dataset is the documented alternative.
pub(crate) const MAX_MEMBER_BYTES: usize = 1024 * 1024;

/// The four sovereignty-ambiguous shapes Natural Earth leaves without an ISO code, mapped to the
/// code `GeoNames` itself uses for places there (step 25's design notes).
///
/// `ISO_A2_EH` already carries `TW` and `XK`, so the table is the safety net for the two shapes
/// Natural Earth leaves blank in both fields — and the documented, deliberate place where a
/// political choice is made rather than inherited from a data file.
pub(crate) const DISPUTED_CODES: &[(&str, &str)] = &[
    ("Taiwan", "TW"),
    ("Kosovo", "XK"),
    ("N. Cyprus", "CY"),
    ("Somaliland", "SO"),
];

/// A quantised coordinate: latitude and longitude in 1e-3 degrees.
pub type Point = (i32, i32);

/// A country's identity, which is all a caller of [`lookup`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    /// ISO 3166-1 alpha-2 code, empty for a shape without one.
    pub code: String,
    /// English name, e.g. `China`.
    pub name: String,
}

/// A country together with its shape: the polygons, each a list of rings, each ring a list of
/// quantised points, the first ring of a polygon being its outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// The country this shape is.
    pub country: Country,
    /// The polygons, in `GeoJSON` order.
    pub polygons: Vec<Vec<Vec<Point>>>,
}

impl Shape {
    /// Whether `(lat, lon)` falls inside this shape.
    ///
    /// The comparison happens in the quantised space, so a coordinate is tested against exactly the
    /// geometry that was stored.
    #[must_use]
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        let y = lat * SCALE;
        let x = lon * SCALE;
        self.polygons.iter().any(|rings| {
            let Some(outline) = rings.first() else {
                return false;
            };
            ring_contains(outline, y, x) && !rings[1..].iter().any(|hole| ring_contains(hole, y, x))
        })
    }
}

/// The even-odd ray-casting test for one ring.
///
/// The ring is treated as closed whether or not its last point repeats its first, and a ring with
/// fewer than three points encloses nothing.
fn ring_contains(ring: &[Point], y: f64, x: f64) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = ring.len() - 1;
    for (index, &(point_y, point_x)) in ring.iter().enumerate() {
        let (point_y, point_x) = (f64::from(point_y), f64::from(point_x));
        let (previous_y, previous_x) = ring[previous];
        let (previous_y, previous_x) = (f64::from(previous_y), f64::from(previous_x));
        if (point_y > y) != (previous_y > y)
            && x < (previous_x - point_x) * (y - point_y) / (previous_y - point_y) + point_x
        {
            inside = !inside;
        }
        previous = index;
    }
    inside
}

// ---------------------------------------------------------------------------------------------
// The build path: GeoJSON in, member out
// ---------------------------------------------------------------------------------------------

/// A parsed Natural Earth document: the shapes and the counts a build report needs.
#[derive(Debug)]
pub struct Parsed {
    /// The countries, in document order.
    pub shapes: Vec<Shape>,
    /// Features dropped because they carry no geometry.
    pub skipped: usize,
    /// The total number of stored points.
    pub points: usize,
}

/// Parses a Natural Earth `admin_0_countries` `GeoJSON` document.
///
/// Strict about the structure it reads — a ring point that is not a number, a coordinate outside
/// the world, a polygon without rings — so a truncated or wrong download fails loudly instead of
/// quietly dropping a country. A feature whose `geometry` is `null` (Natural Earth has a few) is
/// counted in [`Parsed::skipped`] and left out.
pub fn parse_geojson(text: &str) -> std::result::Result<Parsed, String> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| format!("the source is not JSON: {error}"))?;
    let features = document
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| "the document has no `features` array".to_owned())?;

    let mut shapes = Vec::with_capacity(features.len());
    let mut skipped = 0_usize;
    let mut points = 0_usize;
    for (index, feature) in features.iter().enumerate() {
        let name = property(feature, "NAME")
            .ok_or_else(|| format!("feature {index} has no NAME"))?
            .to_owned();
        let code = country_code(feature, &name);
        let polygons = parse_polygons(feature)
            .map_err(|message| format!("feature {index} ({name}): {message}"))?;
        let Some(polygons) = polygons else {
            skipped += 1;
            continue;
        };
        points += polygons
            .iter()
            .flat_map(|rings| rings.iter())
            .map(Vec::len)
            .sum::<usize>();
        shapes.push(Shape {
            country: Country { code, name },
            polygons,
        });
    }
    if shapes.is_empty() {
        return Err("no usable country features in the input".to_owned());
    }
    Ok(Parsed {
        shapes,
        skipped,
        points,
    })
}

/// The ISO code for one feature, or an empty string when it has none.
///
/// `ISO_A2_EH` is Natural Earth's corrected field (it fills in France, Norway, the Indian Ocean
/// Territories and the Ashmore and Cartier Islands, and carries `TW`/`XK`); `ISO_A2` is the older
/// one that leaves those as `-99`; [`DISPUTED_CODES`] covers the four shapes neither field answers.
fn country_code(feature: &Value, name: &str) -> String {
    for field in ["ISO_A2_EH", "ISO_A2"] {
        if let Some(code) =
            property(feature, field).filter(|code| crate::geo::is_country_code(code))
        {
            return code.to_owned();
        }
    }
    DISPUTED_CODES
        .iter()
        .find(|(shape, _)| *shape == name)
        .map_or_else(String::new, |(_, code)| (*code).to_owned())
}

/// One feature's polygons, or `None` when it carries no geometry.
fn parse_polygons(feature: &Value) -> std::result::Result<Option<Vec<Vec<Vec<Point>>>>, String> {
    let Some(geometry) = feature
        .get("geometry")
        .filter(|geometry| !geometry.is_null())
    else {
        return Ok(None);
    };
    let kind = geometry
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "the geometry has no type".to_owned())?;
    let coordinates = geometry
        .get("coordinates")
        .ok_or_else(|| "the geometry has no coordinates".to_owned())?;

    let polygons: Vec<&Value> = match kind {
        "Polygon" => vec![coordinates],
        "MultiPolygon" => coordinates
            .as_array()
            .ok_or_else(|| "a MultiPolygon's coordinates are not an array".to_owned())?
            .iter()
            .collect(),
        other => return Err(format!("`{other}` is not a Polygon or MultiPolygon")),
    };
    let mut parsed = Vec::with_capacity(polygons.len());
    for polygon in polygons {
        let rings = polygon
            .as_array()
            .ok_or_else(|| "a polygon is not an array of rings".to_owned())?;
        if rings.is_empty() {
            return Err("a polygon has no rings".to_owned());
        }
        let mut parsed_rings = Vec::with_capacity(rings.len());
        for ring in rings {
            let points = ring
                .as_array()
                .ok_or_else(|| "a ring is not an array of points".to_owned())?;
            let mut parsed_points = Vec::with_capacity(points.len());
            for point in points {
                parsed_points.push(quantise(point)?);
            }
            parsed_rings.push(parsed_points);
        }
        parsed.push(parsed_rings);
    }
    Ok(Some(parsed))
}

/// One `[lon, lat]` pair, quantised and checked against the world.
fn quantise(point: &Value) -> std::result::Result<Point, String> {
    let pair = point
        .as_array()
        .ok_or_else(|| "a point is not an array".to_owned())?;
    let (Some(lon), Some(lat)) = (pair.first(), pair.get(1)) else {
        return Err("a point has fewer than two coordinates".to_owned());
    };
    let (lon, lat) = (
        lon.as_f64()
            .ok_or_else(|| "a longitude is not a number".to_owned())?,
        lat.as_f64()
            .ok_or_else(|| "a latitude is not a number".to_owned())?,
    );
    if !crate::geo::nominatim::usable_coordinates(lat, lon) {
        return Err(format!("({lat}, {lon}) is outside the world"));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(((lat * SCALE).round() as i32, (lon * SCALE).round() as i32))
}

/// A feature property's non-empty text.
fn property<'a>(feature: &'a Value, name: &str) -> Option<&'a str> {
    feature
        .get("properties")?
        .get(name)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Encodes the member (uncompressed): `magic, u32 count, (country)*`.
pub fn encode(parsed: &Parsed) -> std::result::Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(parsed.points * 8 + parsed.shapes.len() * 16);
    bytes.extend_from_slice(COUNTRIES_MAGIC);
    push_u32(
        &mut bytes,
        u32::try_from(parsed.shapes.len()).map_err(|_| "too many countries".to_owned())?,
    );
    for shape in &parsed.shapes {
        push_text_u8(&mut bytes, &shape.country.code)?;
        push_text_u16(&mut bytes, &shape.country.name)?;
        push_u32(
            &mut bytes,
            u32::try_from(shape.polygons.len()).map_err(|_| "too many polygons".to_owned())?,
        );
        for rings in &shape.polygons {
            push_u32(
                &mut bytes,
                u32::try_from(rings.len()).map_err(|_| "too many rings".to_owned())?,
            );
            for ring in rings {
                push_u32(
                    &mut bytes,
                    u32::try_from(ring.len())
                        .map_err(|_| "too many points in a ring".to_owned())?,
                );
                for &(lat, lon) in ring {
                    push_i32(&mut bytes, lat);
                    push_i32(&mut bytes, lon);
                }
            }
        }
    }
    Ok(bytes)
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_text_u8(bytes: &mut Vec<u8>, text: &str) -> std::result::Result<(), String> {
    let len = u8::try_from(text.len()).map_err(|_| format!("`{text}` is longer than 255 bytes"))?;
    bytes.push(len);
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

fn push_text_u16(bytes: &mut Vec<u8>, text: &str) -> std::result::Result<(), String> {
    let len = u16::try_from(text.len()).map_err(|_| {
        format!(
            "a value of {} bytes exceeds the 65535-byte field",
            text.len()
        )
    })?;
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

/// The provenance record written next to the member, derived from the input alone (no wall clock,
/// so a rebuild of the same input is byte-identical).
#[must_use]
pub fn record_text(parsed: &Parsed, input_sha256: &str) -> String {
    format!(
        "# Natural Earth 1:50m admin-0 countries — the offline country layer (step 25).\n\
         # Public domain (CC0-1.0); see REUSE.toml and LICENSES/CC0-1.0.txt.\n\
         # Rebuild with:\n\
         #   cargo run -p geo-table -- --countries <path or URL> src/geo/data\n\
         source = {OFFICIAL_URL}\n\
         scale = 1e-3\n\
         countries = {}\n\
         points = {}\n\
         input-sha256 = {input_sha256}\n",
        parsed.shapes.len(),
        parsed.points
    )
}

/// One built layer: the member and its record, with the counts a report prints.
#[derive(Debug)]
pub struct CountryCandidate {
    /// The gzipped member.
    pub member: Vec<u8>,
    /// The `COUNTRIES` record.
    pub record: String,
    /// How many countries the layer carries.
    pub countries: usize,
    /// How many points it stores.
    pub points: usize,
    /// How many features were dropped for having no geometry.
    pub skipped: usize,
    /// The SHA-256 of the source document, as the record spells it.
    pub input_sha256: String,
}

/// Reads `source` (a local path or an `http(s)` URL), parses it and builds the member, refusing a
/// member over [`MAX_MEMBER_BYTES`].
pub fn build_candidate(
    source: &str,
    http: &crate::http::HttpClient,
    verbose: u8,
) -> Result<CountryCandidate> {
    let raw = crate::geo::update::read_source(source, http, verbose)?;
    let text = String::from_utf8(raw)
        .map_err(|error| Error::Other(format!("{source} is not UTF-8: {error}")))?;
    let input_sha256 = crate::geo::table::input_sha256(text.as_bytes());
    let parsed =
        parse_geojson(&text).map_err(|message| Error::Other(format!("{source}: {message}")))?;
    let member = crate::geo::table::gzip(
        &encode(&parsed).map_err(|message| Error::Other(format!("{source}: {message}")))?,
    )
    .map_err(|message| Error::Other(format!("{source}: {message}")))?;
    if member.len() > MAX_MEMBER_BYTES {
        return Err(Error::Other(format!(
            "the compressed country layer is {} bytes, over the {MAX_MEMBER_BYTES}-byte budget; \
             build it from the 1:10m dataset instead (the step file's fallback)",
            member.len()
        )));
    }
    // Prove the member decodes before it can replace a working layer; the decoder only exists
    // where the runtime reads the layer, and the builder's canary tests are the check without it.
    #[cfg(feature = "offline-geo")]
    validate(&member)
        .map_err(|message| Error::Other(format!("the built layer does not decode: {message}")))?;
    let record = record_text(&parsed, &input_sha256);
    Ok(CountryCandidate {
        member,
        record,
        countries: parsed.shapes.len(),
        points: parsed.points,
        skipped: parsed.skipped,
        input_sha256,
    })
}

// ---------------------------------------------------------------------------------------------
// The runtime path: the embedded member and the lookup
// ---------------------------------------------------------------------------------------------

/// The committed member, embedded like the city table's pair.
#[cfg(feature = "offline-geo")]
static COUNTRIES_GZ: &[u8] = include_bytes!("data/countries.bin.gz");

/// Decodes the member into shapes, proving every row parses.
#[cfg(feature = "offline-geo")]
pub(crate) fn decode(compressed: &[u8]) -> std::result::Result<Vec<Shape>, String> {
    use crate::geo::table::{Cursor, gunzip};

    let bytes = gunzip(compressed)?;
    let mut cursor = Cursor::new(&bytes);
    cursor.expect(COUNTRIES_MAGIC)?;
    let count = cursor.u32()?;
    let count = usize::try_from(count)
        .map_err(|_| "the country count does not fit this platform".to_owned())?;
    // Each country costs at least its code length, a name length and a polygon count, so a count
    // larger than the bytes that remain cannot be honest; check before reserving anything sized
    // from it.
    if count > cursor.remaining() / 7 {
        return Err(format!(
            "the layer declares {count} countries, more than the {} remaining bytes can hold",
            cursor.remaining()
        ));
    }
    let mut shapes = Vec::with_capacity(count);
    for _ in 0..count {
        let code = cursor.text_u8()?;
        let name = cursor.text_u16()?;
        let polygon_count = cursor.u32()?;
        let mut polygons = Vec::new();
        for _ in 0..polygon_count {
            let ring_count = cursor.u32()?;
            let mut rings = Vec::new();
            for _ in 0..ring_count {
                let point_count = cursor.u32()?;
                let points = usize::try_from(point_count)
                    .map_err(|_| "the point count does not fit this platform".to_owned())?;
                if points > cursor.remaining() / 8 {
                    return Err(format!(
                        "a ring declares {points} points, more than the {} remaining bytes can hold",
                        cursor.remaining()
                    ));
                }
                let mut ring = Vec::with_capacity(points);
                for _ in 0..points {
                    ring.push((cursor.i32()?, cursor.i32()?));
                }
                rings.push(ring);
            }
            polygons.push(rings);
        }
        shapes.push(Shape {
            country: Country { code, name },
            polygons,
        });
    }
    if !cursor.is_empty() {
        return Err(format!(
            "{} trailing bytes after the countries",
            cursor.remaining()
        ));
    }
    Ok(shapes)
}

/// The decoded layer, decoded at most once per process.
#[cfg(feature = "offline-geo")]
static SHAPES: std::sync::LazyLock<std::result::Result<Vec<Shape>, String>> =
    std::sync::LazyLock::new(|| decode(COUNTRIES_GZ));

/// The decoded layer, or the rebuild hint a corrupt embedded member deserves.
#[cfg(feature = "offline-geo")]
fn shapes() -> Result<&'static [Shape]> {
    match &*SHAPES {
        Ok(shapes) => Ok(shapes),
        Err(message) => Err(Error::Other(format!(
            "the bundled country layer is unusable: {message}; rebuild it with `cargo run -p \
             geo-table -- --countries <path or URL> src/geo/data`"
        ))),
    }
}

/// The country `(lat, lon)` falls in, when the layer knows one.
///
/// A coordinate outside every shape — the open ocean, Antarctica's ice shelves, a disputed area
/// neither field names — answers `None`, which is a legitimate "no country here", not an error.
#[cfg(feature = "offline-geo")]
pub fn lookup(lat: f64, lon: f64) -> Result<Option<Country>> {
    Ok(shapes()?
        .iter()
        .find(|shape| shape.contains(lat, lon))
        .map(|shape| shape.country.clone()))
}

/// Decodes a freshly built member without keeping it, so a build proves its own output readable.
#[cfg(feature = "offline-geo")]
pub(crate) fn validate(member: &[u8]) -> std::result::Result<(), String> {
    decode(member).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{DISPUTED_CODES, Shape, country_code, decode, encode, parse_geojson, record_text};

    /// A miniature document with one square country, one with a hole, and one without geometry.
    const SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"NAME":"Square","ISO_A2":"SQ","ISO_A2_EH":"SQ"},
         "geometry":{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}},
        {"type":"Feature","properties":{"NAME":"Ring","ISO_A2":"-99","ISO_A2_EH":"-99"},
         "geometry":{"type":"MultiPolygon","coordinates":[
            [[[10,10],[14,10],[14,14],[10,14],[10,10]],[[11,11],[13,11],[13,13],[11,13],[11,11]]]
         ]}},
        {"type":"Feature","properties":{"NAME":"Nowhere","ISO_A2":"-99"},"geometry":null}
    ]}"#;

    #[test]
    fn a_ring_inside_a_hole_is_not_inside_the_country() {
        let parsed = parse_geojson(SAMPLE).expect("the sample parses");
        assert_eq!(parsed.shapes.len(), 2);
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.points, 15);

        let ring = &parsed.shapes[1];
        assert!(
            ring.contains(10.5, 10.5),
            "between the outline and the hole"
        );
        assert!(!ring.contains(12.0, 12.0), "inside the hole");
        assert!(!ring.contains(9.0, 10.0), "outside altogether");

        let square = &parsed.shapes[0];
        assert!(square.contains(1.0, 1.0));
        assert!(!square.contains(3.0, 1.0));
        // The stored geometry is quantised, so a coordinate is tested against what was stored.
        assert!(square.contains(1.9999, 1.9999));
    }

    #[test]
    fn the_disputed_names_get_their_documented_codes() {
        for (name, code) in DISPUTED_CODES {
            let feature: serde_json::Value = serde_json::from_str(&format!(
                r#"{{"properties":{{"NAME":"{name}","ISO_A2":"-99","ISO_A2_EH":"-99"}}}}"#
            ))
            .expect("the feature parses");
            assert_eq!(country_code(&feature, name), *code, "{name}");
        }
        // The corrected field wins when it has a code, and an unknown name stays empty.
        let feature: serde_json::Value = serde_json::from_str(
            r#"{"properties":{"NAME":"France","ISO_A2":"-99","ISO_A2_EH":"FR"}}"#,
        )
        .expect("the feature parses");
        assert_eq!(country_code(&feature, "France"), "FR");
        let feature: serde_json::Value =
            serde_json::from_str(r#"{"properties":{"NAME":"Siachen Glacier","ISO_A2":"-99"}}"#)
                .expect("the feature parses");
        assert_eq!(country_code(&feature, "Siachen Glacier"), "");
    }

    #[test]
    fn a_wrong_shape_is_refused_rather_than_dropped() {
        for (document, fragment) in [
            ("{}", "no `features` array"),
            (
                r#"{"features":[{"properties":{"NAME":"X"},"geometry":{"type":"Point","coordinates":[0,0]}}]}"#,
                "not a Polygon",
            ),
            (
                r#"{"features":[{"properties":{"NAME":"X"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,"east"]]]}}]}"#,
                "not a number",
            ),
            (
                r#"{"features":[{"properties":{"NAME":"X"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[181,1]]]}}]}"#,
                "outside the world",
            ),
            (
                r#"{"features":[{"properties":{},"geometry":null}]}"#,
                "has no NAME",
            ),
        ] {
            let error = parse_geojson(document).expect_err(&format!("`{document}` is refused"));
            assert!(error.contains(fragment), "`{document}`: {error}");
        }
    }

    /// The member round-trips through the encoder and the decoder.
    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_member_round_trips() {
        let parsed = parse_geojson(SAMPLE).expect("the sample parses");
        let member = crate::geo::table::gzip(&encode(&parsed).expect("the sample encodes"))
            .expect("the sample compresses");
        let decoded = decode(&member).expect("the member decodes");
        assert_eq!(decoded, parsed.shapes);
        assert_eq!(
            decoded[1].country.code, "",
            "the -99 row keeps an empty code"
        );
    }

    /// A truncated or trailing-byte member is a decode error, not a partial layer.
    #[cfg(feature = "offline-geo")]
    #[test]
    fn a_damaged_member_is_refused() {
        let parsed = parse_geojson(SAMPLE).expect("the sample parses");
        let raw = encode(&parsed).expect("the sample encodes");
        let truncated = crate::geo::table::gzip(&raw[..raw.len() - 4]).expect("it compresses");
        assert!(decode(&truncated).is_err());

        let mut extended = raw.clone();
        extended.push(0);
        let with_trailing = crate::geo::table::gzip(&extended).expect("it compresses");
        let error = decode(&with_trailing).expect_err("trailing bytes are refused");
        assert!(error.contains("trailing bytes"), "{error}");
    }

    #[test]
    fn the_record_names_the_source_and_the_counts() {
        let parsed = parse_geojson(SAMPLE).expect("the sample parses");
        let record = record_text(&parsed, "abc123");
        assert!(
            record.contains("source = https://raw.githubusercontent.com/"),
            "{record}"
        );
        assert!(record.contains("countries = 2"), "{record}");
        assert!(record.contains("points = 15"), "{record}");
        assert!(record.contains("input-sha256 = abc123"), "{record}");
        // Deterministic: the same input produces the same record.
        assert_eq!(record, record_text(&parsed, "abc123"));
    }

    /// The committed member is the one the tests pin, and it answers the world.
    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_committed_layer_answers_known_coordinates() {
        let beijing = super::lookup(39.9042, 116.4074)
            .expect("the layer decodes")
            .expect("Beijing is in China");
        assert_eq!(beijing.code, "CN");
        assert_eq!(beijing.name, "China");

        // A hole: Lesotho is its own country, and South Africa's shape must not swallow it.
        let lesotho = super::lookup(-29.31, 27.48)
            .expect("the layer decodes")
            .expect("Maseru is in Lesotho");
        assert_eq!(lesotho.code, "LS");

        // The open ocean has no country at all.
        assert_eq!(
            super::lookup(-40.0, -140.0).expect("the layer decodes"),
            None
        );

        // The shapes Natural Earth leaves without a code get the documented one.
        for (lat, lon, code) in [
            (25.03, 121.56, "TW"),
            (35.19, 33.38, "CY"),
            (42.66, 21.17, "XK"),
            (9.56, 44.07, "SO"),
        ] {
            let country = super::lookup(lat, lon)
                .expect("the layer decodes")
                .unwrap_or_else(|| panic!("({lat}, {lon}) is inside a shape"));
            assert_eq!(country.code, code, "({lat}, {lon}): {country:?}");
        }
    }

    /// The committed member stays inside the budget the step file names.
    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_committed_layer_is_within_the_size_budget() {
        assert!(
            super::COUNTRIES_GZ.len() <= super::MAX_MEMBER_BYTES,
            "{} bytes",
            super::COUNTRIES_GZ.len()
        );
        assert!(
            super::COUNTRIES_GZ.len() < 512 * 1024,
            "the layer grew past the recorded 408 KiB: {} bytes",
            super::COUNTRIES_GZ.len()
        );
    }

    /// A shape's `contains` never panics on a degenerate ring.
    #[test]
    fn a_degenerate_ring_encloses_nothing() {
        let shape = Shape {
            country: super::Country {
                code: "XX".to_owned(),
                name: "Empty".to_owned(),
            },
            polygons: vec![vec![Vec::new()], vec![vec![(0, 0), (1, 1)]]],
        };
        assert!(!shape.contains(0.0, 0.0));
    }
}
