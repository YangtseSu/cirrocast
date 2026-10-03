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
        "Polygon" => Some(polygon_contains(coordinates, lat, lon)),
        // A MultiPolygon is an array of Polygons; containing any one of them is enough.
        "MultiPolygon" => Some(
            coordinates
                .as_array()?
                .iter()
                .any(|polygon| polygon_contains(polygon, lat, lon)),
        ),
        _ => None,
    }
}

/// Whether one `GeoJSON` polygon (`[[lon, lat], …]` rings; the first is the exterior, the rest are
/// holes) contains the point.
#[must_use]
fn polygon_contains(polygon: &serde_json::Value, lat: f64, lon: f64) -> bool {
    let Some(rings) = polygon.as_array() else {
        return false;
    };
    let Some((exterior, holes)) = rings.split_first() else {
        return false;
    };
    if !ring_contains(exterior, lat, lon) {
        return false;
    }
    // A point inside a hole is outside the polygon; a point on a hole's edge is ambiguous and
    // treated as outside.
    !holes.iter().any(|hole| ring_contains(hole, lat, lon))
}

/// One ring of `[lon, lat]` pairs, closed or not (the ray cast treats it as closed).
fn ring_contains(ring: &serde_json::Value, lat: f64, lon: f64) -> bool {
    let Some(points) = ring.as_array() else {
        return false;
    };
    let pairs: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|point| {
            let pair = point.as_array()?;
            let x = pair.first()?.as_f64()?;
            let y = pair.get(1)?.as_f64()?;
            Some((x, y))
        })
        .collect();
    if pairs.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = pairs.len() - 1;
    for i in 0..pairs.len() {
        let (xi, yi) = pairs[i];
        let (xj, yj) = pairs[j];
        // The classic even-odd test, with the `>`/`<=` asymmetry that keeps a vertex on a
        // horizontal scanline from being counted twice.
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
/// `None` when fewer than three pairs parse; an unclosed ring is treated as closed.
#[must_use]
pub fn cap_polygon_contains(polygon: &str, lat: f64, lon: f64) -> Option<bool> {
    let pairs: Vec<(f64, f64)> = polygon.split_whitespace().filter_map(parse_pair).collect();
    if pairs.len() < 3 {
        return None;
    }
    let mut inside = false;
    let mut j = pairs.len() - 1;
    for i in 0..pairs.len() {
        let (yi, xi) = pairs[i];
        let (yj, xj) = pairs[j];
        if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    Some(inside)
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
}
