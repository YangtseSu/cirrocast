// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Point-in-area tests for the alert sources that filter client-side.
//!
//! WMO `SWIC` runs the test server-side (its WFS query is an `INTERSECTS` against the point), but
//! `MeteoAlarm`'s EDR features and `FPAS`'s CAP areas arrive whole, and an alert whose area does not
//! contain the requested point is dropped here. Two shapes occur:
//!
//! * **`GeoJSON` geometry** (`Polygon`/`MultiPolygon`, coordinates `[lon, lat]`) in `MeteoAlarm`'s
//!   feature collection and `FPAS`'s `/alert/<uuid>` documents;
//! * **CAP area strings** (`polygon` = `lat,lon lat,lon …`, `circle` = `lat,lon radius-km`) in CAP
//!   documents.
//!
//! The polygon test is the even-odd ray cast; a point exactly on an edge may fall either way,
//! which is the accepted behaviour for a warning boundary (an alert for the exact edge is rare and
//! a false negative there is no worse than the upstream's own rounding).

/// The mean Earth radius in kilometres, for the circle test.
const EARTH_RADIUS_KM: f64 = 6_371.008_8;

/// Whether `geometry` (a `GeoJSON` `Polygon` or `MultiPolygon` value) contains the point.
///
/// `None` when the geometry is missing, `null` or a shape this test does not know: the caller
/// decides whether an untestable area is kept (the honest default) or dropped.
#[must_use]
pub fn geojson_contains(geometry: Option<&serde_json::Value>, lat: f64, lon: f64) -> Option<bool> {
    let geometry = geometry?;
    let kind = geometry.get("type")?.as_str()?;
    let coordinates = geometry.get("coordinates")?;
    match kind {
        "Polygon" => polygon_contains(coordinates, lat, lon),
        // A MultiPolygon is an array of Polygons; containing any one of them is enough, but an
        // untestable member makes the whole answer untestable rather than a confident `false`.
        "MultiPolygon" => {
            let polygons = coordinates.as_array()?;
            let mut result = Some(false);
            for polygon in polygons {
                match polygon_contains(polygon, lat, lon) {
                    Some(true) => return Some(true),
                    None => result = None,
                    Some(false) => {}
                }
            }
            result
        }
        _ => None,
    }
}

/// Whether one `GeoJSON` polygon (`[[lon, lat], …]` rings; the first is the exterior, the rest are
/// holes) contains the point.
///
/// `None` when the polygon is structurally broken or a ring holds a vertex this test cannot read:
/// the tested shape must be the issued shape, so a partially parsable ring is untestable rather
/// than silently tested without its bad vertices.
#[must_use]
fn polygon_contains(polygon: &serde_json::Value, lat: f64, lon: f64) -> Option<bool> {
    let rings = polygon.as_array()?;
    let (exterior, holes) = rings.split_first()?;
    if !ring_contains(exterior, lat, lon)? {
        return Some(false);
    }
    // A point inside a hole is outside the polygon; a point on a hole's edge is ambiguous and
    // treated as outside.
    for hole in holes {
        if ring_contains(hole, lat, lon)? {
            return Some(false);
        }
    }
    Some(true)
}

/// One ring of `[lon, lat]` pairs, closed or not (the ray cast treats it as closed).
///
/// `None` when the ring is not an array of readable vertices or carries fewer than three of them.
fn ring_contains(ring: &serde_json::Value, lat: f64, lon: f64) -> Option<bool> {
    let points = ring.as_array()?;
    let mut pairs = Vec::with_capacity(points.len());
    for point in points {
        let pair = point.as_array()?;
        let x = pair.first()?.as_f64()?;
        let y = pair.get(1)?.as_f64()?;
        pairs.push((x, y));
    }
    if pairs.len() < 3 {
        return None;
    }
    Some(cast_contains(&pairs, lat, lon))
}

/// The even-odd ray cast, with the longitudes unwrapped across the antimeridian first.
///
/// A ring that crosses ±180° stores its straddling edges at +180 and −180, so the raw cast sees no
/// crossing; the vertices are unwrapped into one continuous span and the point is tested in the
/// ring's own frame (`lon`, `lon ± 360°`).
fn cast_contains(pairs: &[(f64, f64)], lat: f64, lon: f64) -> bool {
    let unwrapped = unwrap_longitudes(pairs);
    [lon, lon + 360.0, lon - 360.0]
        .into_iter()
        .any(|x| even_odd(&unwrapped, lat, x))
}

/// A ring of `(lon, lat)` pairs with each longitude adjusted by ±360° to stay within 180° of its
/// predecessor, so a ring crossing the antimeridian becomes one continuous coordinate span.
fn unwrap_longitudes(pairs: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(pairs.len());
    for &(mut lon, lat) in pairs {
        if let Some(&(previous, _)) = out.last() {
            while lon - previous > 180.0 {
                lon -= 360.0;
            }
            while lon - previous < -180.0 {
                lon += 360.0;
            }
        }
        out.push((lon, lat));
    }
    out
}

/// The classic even-odd test on already unwrapped longitudes.
fn even_odd(pairs: &[(f64, f64)], lat: f64, lon: f64) -> bool {
    let mut inside = false;
    let mut j = pairs.len() - 1;
    for i in 0..pairs.len() {
        let (xi, yi) = pairs[i];
        let (xj, yj) = pairs[j];
        // The `>`/`<=` asymmetry keeps a vertex on a horizontal scanline from being counted twice.
        if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Whether the CAP `circle` string (`lat,lon radius-km`) contains the point.
///
/// `None` when the string does not parse; the caller decides what an unparsable area means.
#[must_use]
pub fn cap_circle_contains(circle: &str, lat: f64, lon: f64) -> Option<bool> {
    let mut parts = circle.split_whitespace();
    let centre = parts.next()?;
    let radius_km: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let (centre_lat, centre_lon) = parse_pair(centre)?;
    Some(haversine_km(lat, lon, centre_lat, centre_lon) <= radius_km)
}

/// Whether the CAP `polygon` string (`lat,lon lat,lon …`) contains the point.
///
/// `None` when any vertex does not parse or fewer than three pairs are present: an unparsable
/// vertex means the tested shape would differ from the issued one, so the geometry is untestable
/// and the caller keeps the alert. An unclosed ring is treated as closed, and a ring crossing the
/// antimeridian is unwrapped like a `GeoJSON` ring.
#[must_use]
pub fn cap_polygon_contains(polygon: &str, lat: f64, lon: f64) -> Option<bool> {
    let mut pairs = Vec::new();
    for token in polygon.split_whitespace() {
        let (vertex_lat, vertex_lon) = parse_pair(token)?;
        pairs.push((vertex_lon, vertex_lat));
    }
    if pairs.len() < 3 {
        return None;
    }
    Some(cast_contains(&pairs, lat, lon))
}

/// One `lat,lon` pair from a CAP area string.
fn parse_pair(text: &str) -> Option<(f64, f64)> {
    let (lat, lon) = text.split_once(',')?;
    let lat = lat.trim().parse().ok()?;
    let lon = lon.trim().parse().ok()?;
    Some((lat, lon))
}

/// The great-circle distance in kilometres between two points.
#[must_use]
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();
    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{cap_circle_contains, cap_polygon_contains, geojson_contains, haversine_km};

    /// A 1°×1° square around (39.9, 116.4): `[lon, lat]` pairs, closed.
    fn square() -> serde_json::Value {
        json!({
            "type": "Polygon",
            "coordinates": [[
                [116.0, 39.5], [116.9, 39.5], [116.9, 40.3], [116.0, 40.3], [116.0, 39.5]
            ]]
        })
    }

    #[test]
    fn a_polygon_contains_its_interior_and_not_its_outside() {
        let geometry = square();
        assert_eq!(geojson_contains(Some(&geometry), 39.9, 116.4), Some(true));
        assert_eq!(geojson_contains(Some(&geometry), 40.0, 116.5), Some(true));
        assert_eq!(geojson_contains(Some(&geometry), 41.0, 116.4), Some(false));
        assert_eq!(geojson_contains(Some(&geometry), 39.9, 120.0), Some(false));
    }

    #[test]
    fn a_hole_is_outside_the_polygon() {
        let geometry = json!({
            "type": "Polygon",
            "coordinates": [
                [[116.0, 39.5], [117.0, 39.5], [117.0, 40.5], [116.0, 40.5], [116.0, 39.5]],
                [[116.4, 39.9], [116.6, 39.9], [116.6, 40.1], [116.4, 40.1], [116.4, 39.9]]
            ]
        });
        assert_eq!(geojson_contains(Some(&geometry), 40.0, 116.5), Some(false));
        assert_eq!(geojson_contains(Some(&geometry), 39.7, 116.2), Some(true));
    }

    #[test]
    fn a_multipolygon_contains_a_point_in_any_part() {
        let geometry = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]],
                [[[116.0, 39.0], [117.0, 39.0], [117.0, 40.0], [116.0, 40.0], [116.0, 39.0]]]
            ]
        });
        assert_eq!(geojson_contains(Some(&geometry), 39.5, 116.5), Some(true));
        assert_eq!(geojson_contains(Some(&geometry), 0.5, 0.5), Some(true));
        assert_eq!(geojson_contains(Some(&geometry), -1.0, -1.0), Some(false));
    }

    #[test]
    fn an_unknown_or_missing_geometry_is_untestable() {
        assert_eq!(geojson_contains(None, 0.0, 0.0), None);
        assert_eq!(geojson_contains(Some(&json!(null)), 0.0, 0.0), None);
        assert_eq!(
            geojson_contains(
                Some(&json!({"type": "Point", "coordinates": [0.0, 0.0]})),
                0.0,
                0.0
            ),
            None
        );
    }

    #[test]
    fn cap_polygons_use_lat_lon_pairs() {
        let polygon = "39.5,116.0 39.5,116.9 40.3,116.9 40.3,116.0 39.5,116.0";
        assert_eq!(cap_polygon_contains(polygon, 39.9, 116.4), Some(true));
        assert_eq!(cap_polygon_contains(polygon, 41.0, 116.4), Some(false));
        assert_eq!(
            cap_polygon_contains("39.5,116.0 39.5,116.9", 39.9, 116.4),
            None
        );
    }

    #[test]
    fn cap_circles_measure_haversine_distance() {
        // Roughly 111 km per degree of latitude at this scale.
        let circle = "39.9,116.4 50";
        assert_eq!(cap_circle_contains(circle, 39.9, 116.4), Some(true));
        assert_eq!(cap_circle_contains(circle, 40.2, 116.4), Some(true));
        assert_eq!(cap_circle_contains(circle, 40.5, 116.4), Some(false));
        assert_eq!(cap_circle_contains("nonsense", 0.0, 0.0), None);
        assert!(haversine_km(0.0, 0.0, 1.0, 0.0).abs() > 110.0);
        assert!(haversine_km(0.0, 0.0, 1.0, 0.0).abs() < 112.0);
    }

    #[test]
    fn an_antimeridian_ring_contains_a_point_on_either_side() {
        // A Fiji/Kiribati-style box straddling 180°: 177…179 and −179…−178.
        let geometry = json!({
            "type": "Polygon",
            "coordinates": [[
                [177.0, -17.0], [179.0, -17.0], [-179.0, -18.5], [177.0, -18.5], [177.0, -17.0]
            ]]
        });
        // Inside near +180 (the raw cast would call this outside, since 178.45 < every edge).
        assert_eq!(geojson_contains(Some(&geometry), -17.5, 178.45), Some(true));
        // The same point spelled as a negative longitude is inside too.
        assert_eq!(
            geojson_contains(Some(&geometry), -17.5, -181.55),
            Some(true)
        );
        // A point on the other side of the globe is outside.
        assert_eq!(geojson_contains(Some(&geometry), 0.0, 0.0), Some(false));
        assert_eq!(geojson_contains(Some(&geometry), -30.0, 90.0), Some(false));

        // The CAP polygon spelling of the same ring.
        let cap = "-17.0,177.0 -17.0,179.0 -18.5,-179.0 -18.5,177.0 -17.0,177.0";
        assert_eq!(cap_polygon_contains(cap, -17.5, 178.45), Some(true));
        assert_eq!(cap_polygon_contains(cap, -17.5, -181.55), Some(true));
        assert_eq!(cap_polygon_contains(cap, 0.0, 0.0), Some(false));
    }

    #[test]
    fn a_partially_parsable_ring_is_untestable_not_dropped() {
        // One vertex is not a coordinate pair: the tested shape must equal the issued one.
        let broken_json = json!({
            "type": "Polygon",
            "coordinates": [[
                [116.0, 39.5], ["oops", 39.5], [116.9, 40.3], [116.0, 40.3], [116.0, 39.5]
            ]]
        });
        assert_eq!(geojson_contains(Some(&broken_json), 39.9, 116.4), None);

        // A structurally broken ring (too few vertices) is untestable, not a confident `false`.
        let too_few = json!({
            "type": "Polygon",
            "coordinates": [[[116.0, 39.5], [116.9, 39.5]]]
        });
        assert_eq!(geojson_contains(Some(&too_few), 39.9, 116.4), None);

        // A MultiPolygon with one good and one broken member is untestable where the good member
        // does not answer.
        let mixed = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]],
                [[[116.0, 39.5], ["oops", 39.5], [116.9, 40.3], [116.0, 40.3]]]
            ]
        });
        assert_eq!(geojson_contains(Some(&mixed), 0.5, 0.5), Some(true));
        assert_eq!(geojson_contains(Some(&mixed), 39.9, 116.4), None);

        // A CAP polygon with an unparsable vertex is untestable too.
        assert_eq!(
            cap_polygon_contains("39.5,116.0 39.5,oops 40.3,116.9 40.3,116.0", 39.9, 116.4),
            None
        );
    }
}
