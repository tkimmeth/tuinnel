// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// globe.rs — Spinning braille globe with orthographic projection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Renders a spinning Earth using ratatui's Canvas with braille markers.
// World map data is generated at startup from simplified continent polygons
// using point-in-polygon testing. The globe uses orthographic projection
// (looks like viewing Earth from space).

use std::f64::consts::PI;

/// A simplified polygon defined by lat/lon vertices.
struct Polygon {
    vertices: &'static [(f64, f64)], // (lat, lon) pairs
}

impl Polygon {
    /// Ray-casting point-in-polygon test.
    fn contains(&self, lat: f64, lon: f64) -> bool {
        let n = self.vertices.len();
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (yi, xi) = self.vertices[i];
            let (yj, xj) = self.vertices[j];
            if ((yi > lat) != (yj > lat))
                && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi)
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

// ── Continent Polygon Data ──────────────────────────────────────────────────
//
// Each continent is defined as one or more polygons with (lat, lon) vertices.
// These are simplified outlines optimized for terminal-resolution rendering.
// Accuracy is ~5° — enough for recognizable shapes on a braille globe.

static NORTH_AMERICA: [(f64, f64); 20] = [
    (48.0, -128.0), (55.0, -132.0), (60.0, -142.0), (64.0, -168.0),
    (72.0, -168.0), (76.0, -120.0), (73.0, -85.0), (62.0, -63.0),
    (48.0, -55.0), (44.0, -64.0), (42.0, -70.0), (35.0, -76.0),
    (30.0, -82.0), (25.0, -82.0), (25.0, -98.0), (20.0, -103.0),
    (32.0, -118.0), (38.0, -122.0), (42.0, -124.0), (48.0, -125.0),
];

static MEXICO_CENTRAL_AMERICA: [(f64, f64); 10] = [
    (32.0, -118.0), (25.0, -98.0), (25.0, -82.0), (20.0, -88.0),
    (15.0, -92.0), (10.0, -84.0), (8.0, -78.0), (15.0, -88.0),
    (20.0, -103.0), (28.0, -112.0),
];

static GREENLAND: [(f64, f64); 8] = [
    (60.0, -48.0), (66.0, -55.0), (72.0, -58.0), (78.0, -68.0),
    (83.0, -42.0), (80.0, -20.0), (72.0, -22.0), (64.0, -40.0),
];

static SOUTH_AMERICA: [(f64, f64); 14] = [
    (12.0, -72.0), (8.0, -62.0), (5.0, -52.0), (0.0, -50.0),
    (-5.0, -35.0), (-15.0, -40.0), (-23.0, -42.0), (-33.0, -52.0),
    (-55.0, -68.0), (-52.0, -72.0), (-40.0, -74.0), (-20.0, -70.0),
    (-5.0, -78.0), (5.0, -77.0),
];

static EUROPE: [(f64, f64); 14] = [
    (36.0, -8.0), (43.0, -10.0), (48.0, -5.0), (51.0, 2.0),
    (54.0, 8.0), (56.0, 12.0), (62.0, 5.0), (70.0, 20.0),
    (72.0, 30.0), (60.0, 30.0), (54.0, 22.0), (48.0, 18.0),
    (42.0, 28.0), (38.0, 0.0),
];

static BRITISH_ISLES: [(f64, f64); 6] = [
    (50.0, -6.0), (52.0, -10.0), (56.0, -8.0), (58.0, -5.0),
    (55.0, 0.0), (51.0, 1.0),
];

static SCANDINAVIA: [(f64, f64); 8] = [
    (56.0, 8.0), (58.0, 6.0), (63.0, 5.0), (70.0, 16.0),
    (72.0, 28.0), (68.0, 30.0), (62.0, 18.0), (58.0, 12.0),
];

static AFRICA: [(f64, f64); 16] = [
    (37.0, -2.0), (35.0, 10.0), (32.0, 32.0), (22.0, 36.0),
    (12.0, 44.0), (2.0, 42.0), (-2.0, 42.0), (-12.0, 40.0),
    (-26.0, 33.0), (-35.0, 20.0), (-34.0, 18.0), (-20.0, 12.0),
    (-5.0, 8.0), (5.0, 2.0), (5.0, -8.0), (15.0, -17.0),
];

static RUSSIA_ASIA: [(f64, f64); 14] = [
    (55.0, 30.0), (60.0, 30.0), (72.0, 30.0), (78.0, 60.0),
    (76.0, 100.0), (72.0, 140.0), (68.0, 170.0), (62.0, 175.0),
    (55.0, 160.0), (50.0, 140.0), (52.0, 105.0), (50.0, 80.0),
    (45.0, 55.0), (48.0, 40.0),
];

static MIDDLE_EAST: [(f64, f64); 10] = [
    (38.0, 28.0), (42.0, 42.0), (40.0, 50.0), (36.0, 55.0),
    (25.0, 58.0), (15.0, 45.0), (12.0, 44.0), (22.0, 36.0),
    (32.0, 32.0), (36.0, 36.0),
];

static INDIA: [(f64, f64); 10] = [
    (35.0, 72.0), (28.0, 68.0), (22.0, 70.0), (15.0, 74.0),
    (8.0, 78.0), (12.0, 80.0), (18.0, 84.0), (22.0, 90.0),
    (28.0, 92.0), (35.0, 78.0),
];

static CHINA_EAST_ASIA: [(f64, f64); 12] = [
    (50.0, 80.0), (52.0, 105.0), (50.0, 120.0), (48.0, 135.0),
    (42.0, 130.0), (35.0, 128.0), (30.0, 122.0), (22.0, 115.0),
    (20.0, 108.0), (22.0, 100.0), (28.0, 98.0), (40.0, 75.0),
];

static SE_ASIA: [(f64, f64); 8] = [
    (22.0, 100.0), (20.0, 108.0), (18.0, 106.0), (10.0, 106.0),
    (2.0, 104.0), (0.0, 98.0), (8.0, 98.0), (15.0, 98.0),
];

static JAPAN: [(f64, f64); 6] = [
    (31.0, 131.0), (34.0, 132.0), (36.0, 136.0), (40.0, 140.0),
    (44.0, 145.0), (38.0, 138.0),
];

static INDONESIA: [(f64, f64); 8] = [
    (-2.0, 96.0), (2.0, 104.0), (2.0, 110.0), (0.0, 118.0),
    (-3.0, 120.0), (-8.0, 115.0), (-8.0, 108.0), (-6.0, 104.0),
];

static AUSTRALIA: [(f64, f64); 12] = [
    (-12.0, 130.0), (-12.0, 136.0), (-15.0, 140.0), (-18.0, 146.0),
    (-25.0, 153.0), (-30.0, 153.0), (-38.0, 148.0), (-38.0, 140.0),
    (-32.0, 133.0), (-32.0, 115.0), (-22.0, 114.0), (-14.0, 126.0),
];

static NEW_ZEALAND: [(f64, f64); 6] = [
    (-35.0, 173.0), (-38.0, 176.0), (-42.0, 174.0), (-47.0, 168.0),
    (-44.0, 168.0), (-38.0, 172.0),
];

static ANTARCTICA: [(f64, f64); 8] = [
    (-65.0, -60.0), (-70.0, -20.0), (-68.0, 40.0), (-70.0, 80.0),
    (-68.0, 140.0), (-72.0, 170.0), (-76.0, -130.0), (-70.0, -80.0),
];

fn continent_polygons() -> Vec<Polygon> {
    vec![
        Polygon { vertices: &NORTH_AMERICA },
        Polygon { vertices: &MEXICO_CENTRAL_AMERICA },
        Polygon { vertices: &GREENLAND },
        Polygon { vertices: &SOUTH_AMERICA },
        Polygon { vertices: &EUROPE },
        Polygon { vertices: &BRITISH_ISLES },
        Polygon { vertices: &SCANDINAVIA },
        Polygon { vertices: &AFRICA },
        Polygon { vertices: &RUSSIA_ASIA },
        Polygon { vertices: &MIDDLE_EAST },
        Polygon { vertices: &INDIA },
        Polygon { vertices: &CHINA_EAST_ASIA },
        Polygon { vertices: &SE_ASIA },
        Polygon { vertices: &JAPAN },
        Polygon { vertices: &INDONESIA },
        Polygon { vertices: &AUSTRALIA },
        Polygon { vertices: &NEW_ZEALAND },
        Polygon { vertices: &ANTARCTICA },
    ]
}

// ── World Map Bitmap ────────────────────────────────────────────────────────

/// Pre-computed world map bitmap at 1° resolution.
pub struct WorldMap {
    /// Row-major bool grid. Rows: lat 90°N (row 0) to 90°S (row 179).
    /// Cols: lon -180° (col 0) to 179°E (col 359).
    data: Vec<bool>,
    cols: usize,
    rows: usize,
}

impl WorldMap {
    /// Generate the world map by testing each grid cell against continent polygons.
    pub fn generate() -> Self {
        let cols = 360;
        let rows = 180;
        let polygons = continent_polygons();
        let mut data = vec![false; cols * rows];

        for row in 0..rows {
            let lat = 89.5 - row as f64;
            for col in 0..cols {
                let lon = -179.5 + col as f64;
                for poly in &polygons {
                    if poly.contains(lat, lon) {
                        data[row * cols + col] = true;
                        break;
                    }
                }
            }
        }

        Self { data, cols, rows }
    }

    /// Check if a (lat, lon) point is land.
    pub fn is_land(&self, lat: f64, lon: f64) -> bool {
        let row = ((89.5 - lat) as usize).min(self.rows - 1);
        // Wrap longitude
        let lon_wrapped = ((lon % 360.0) + 360.0) % 360.0 - 180.0;
        let col = ((lon_wrapped + 179.5) as usize).min(self.cols - 1);
        self.data[row * self.cols + col]
    }
}

// ── Globe Projection ────────────────────────────────────────────────────────

/// A point to render on the globe canvas.
pub struct GlobePoint {
    pub x: f64,
    pub y: f64,
}

/// Compute visible land points for the current rotation angle.
/// Returns points in [-1, 1] canvas coordinates.
pub fn project_land(world_map: &WorldMap, rotation_deg: f64) -> Vec<GlobePoint> {
    let mut points = Vec::with_capacity(4000);
    let rot = rotation_deg.to_radians();

    let step = 1;
    for row in (0..world_map.rows).step_by(step) {
        let lat = 89.5 - row as f64;
        let lat_rad = lat.to_radians();
        let cos_lat = lat_rad.cos();
        let sin_lat = lat_rad.sin();

        for col in (0..world_map.cols).step_by(step) {
            if !world_map.data[row * world_map.cols + col] {
                continue;
            }

            let lon = -179.5 + col as f64;
            let lon_rad = (lon.to_radians()) - rot;

            // Visibility check: point is on the front hemisphere
            let cos_lon = lon_rad.cos();
            if cos_lat * cos_lon <= 0.0 {
                continue;
            }

            // Orthographic projection
            let x = cos_lat * lon_rad.sin();
            let y = sin_lat;

            points.push(GlobePoint { x, y });
        }
    }

    points
}

/// Compute points along the globe outline circle.
pub fn globe_outline() -> Vec<(f64, f64)> {
    let n = 128;
    (0..=n)
        .map(|i| {
            let angle = 2.0 * PI * (i as f64) / (n as f64);
            (angle.cos(), angle.sin())
        })
        .collect()
}

/// Project a single (lat, lon) point onto the globe.
/// Returns Some((x, y)) if visible, None if on the back hemisphere.
pub fn project_point(lat: f64, lon: f64, rotation_deg: f64) -> Option<(f64, f64)> {
    let lat_rad = lat.to_radians();
    let rot = rotation_deg.to_radians();
    let lon_rad = lon.to_radians() - rot;

    let cos_lat = lat_rad.cos();
    let cos_lon = lon_rad.cos();

    if cos_lat * cos_lon <= 0.0 {
        return None;
    }

    let x = cos_lat * lon_rad.sin();
    let y = lat_rad.sin();
    Some((x, y))
}

/// Compute points along a great circle arc between two (lat, lon) points.
/// Returns visible projected points.
pub fn great_circle_arc(
    lat1: f64, lon1: f64,
    lat2: f64, lon2: f64,
    rotation_deg: f64,
    segments: usize,
) -> Vec<(f64, f64)> {
    let lat1r = lat1.to_radians();
    let lon1r = lon1.to_radians();
    let lat2r = lat2.to_radians();
    let lon2r = lon2.to_radians();

    // Convert to Cartesian
    let x1 = lat1r.cos() * lon1r.cos();
    let y1 = lat1r.cos() * lon1r.sin();
    let z1 = lat1r.sin();

    let x2 = lat2r.cos() * lon2r.cos();
    let y2 = lat2r.cos() * lon2r.sin();
    let z2 = lat2r.sin();

    // Angular distance
    let dot = (x1 * x2 + y1 * y2 + z1 * z2).clamp(-1.0, 1.0);
    let omega = dot.acos();

    if omega.abs() < 1e-10 {
        return Vec::new();
    }

    let sin_omega = omega.sin();

    let mut points = Vec::new();
    for i in 0..=segments {
        let t = i as f64 / segments as f64;
        let a = ((1.0 - t) * omega).sin() / sin_omega;
        let b = (t * omega).sin() / sin_omega;

        let x = a * x1 + b * x2;
        let y = a * y1 + b * y2;
        let z = a * z1 + b * z2;

        let lat = z.asin().to_degrees();
        let lon = y.atan2(x).to_degrees();

        if let Some(pt) = project_point(lat, lon, rotation_deg) {
            points.push(pt);
        }
    }

    points
}
