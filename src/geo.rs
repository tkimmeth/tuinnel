// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// geo.rs — City/country coordinate lookup for globe rendering
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Bundled lookup table mapping ProtonVPN server locations to lat/lon.
// No network dependency — coordinates are hardcoded.

/// (city_code, city_name, country_code, latitude, longitude)
static CITIES: &[(&str, &str, &str, f64, f64)] = &[
    // United States — city codes
    ("NY", "New York", "US", 40.71, -74.01),
    ("LA", "Los Angeles", "US", 34.05, -118.24),
    ("CH", "Chicago", "US", 41.88, -87.63),
    ("MI", "Miami", "US", 25.76, -80.19),
    ("DA", "Dallas", "US", 32.78, -96.80),
    ("SE", "Seattle", "US", 47.61, -122.33),
    ("SF", "San Francisco", "US", 37.77, -122.42),
    ("DC", "Washington DC", "US", 38.91, -77.04),
    ("AT", "Atlanta", "US", 33.75, -84.39),
    ("DE", "Denver", "US", 39.74, -104.99),
    ("PX", "Phoenix", "US", 33.45, -112.07),
    ("SL", "Salt Lake City", "US", 40.76, -111.89),
    ("HO", "Houston", "US", 29.76, -95.37),
    ("MN", "Minneapolis", "US", 44.98, -93.27),
    // United States — state codes (ProtonVPN uses these)
    ("FL", "Florida", "US", 25.76, -80.19),
    ("CA", "California", "US", 34.05, -118.24),
    ("TX", "Texas", "US", 32.78, -96.80),
    ("IL", "Illinois", "US", 41.88, -87.63),
    ("GA", "Georgia", "US", 33.75, -84.39),
    ("WA", "Washington", "US", 47.61, -122.33),
    ("CO", "Colorado", "US", 39.74, -104.99),
    ("AZ", "Arizona", "US", 33.45, -112.07),
    ("UT", "Utah", "US", 40.76, -111.89),
    ("VA", "Virginia", "US", 38.91, -77.04),
    ("NJ", "New Jersey", "US", 40.71, -74.01),
    ("PA", "Pennsylvania", "US", 39.95, -75.17),
    ("OH", "Ohio", "US", 39.96, -82.99),
    ("OR", "Oregon", "US", 45.52, -122.68),
    ("NC", "North Carolina", "US", 35.78, -78.64),
    ("SC", "South Carolina", "US", 32.78, -79.93),
    ("MA", "Massachusetts", "US", 42.36, -71.06),
    ("MD", "Maryland", "US", 39.29, -76.61),
    ("CT", "Connecticut", "US", 41.76, -72.68),
    // Canada
    ("TO", "Toronto", "CA", 43.65, -79.38),
    ("MO", "Montreal", "CA", 45.50, -73.57),
    ("VA", "Vancouver", "CA", 49.28, -123.12),
    // United Kingdom
    ("LO", "London", "UK", 51.51, -0.13),
    ("MA", "Manchester", "UK", 53.48, -2.24),
    ("ED", "Edinburgh", "UK", 55.95, -3.19),
    // Netherlands
    ("AM", "Amsterdam", "NL", 52.37, 4.90),
    ("RO", "Rotterdam", "NL", 51.92, 4.48),
    // Germany
    ("FR", "Frankfurt", "DE", 50.11, 8.68),
    ("BE", "Berlin", "DE", 52.52, 13.41),
    ("MU", "Munich", "DE", 48.14, 11.58),
    // France
    ("PA", "Paris", "FR", 48.86, 2.35),
    ("MA", "Marseille", "FR", 43.30, 5.37),
    // Switzerland
    ("ZU", "Zurich", "CH", 47.38, 8.54),
    ("GE", "Geneva", "CH", 46.20, 6.14),
    // Scandinavia
    ("ST", "Stockholm", "SE", 59.33, 18.07),
    ("MA", "Malmo", "SE", 55.61, 13.00),
    ("OS", "Oslo", "NO", 59.91, 10.75),
    ("CO", "Copenhagen", "DK", 55.68, 12.57),
    ("HE", "Helsinki", "FI", 60.17, 24.94),
    // Iberia
    ("MA", "Madrid", "ES", 40.42, -3.70),
    ("BA", "Barcelona", "ES", 41.39, 2.17),
    ("LI", "Lisbon", "PT", 38.72, -9.14),
    // Italy
    ("RO", "Rome", "IT", 41.90, 12.50),
    ("MI", "Milan", "IT", 45.46, 9.19),
    // Central Europe
    ("VI", "Vienna", "AT", 48.21, 16.37),
    ("WA", "Warsaw", "PL", 52.23, 21.01),
    ("KR", "Krakow", "PL", 50.06, 19.94),
    ("PR", "Prague", "CZ", 50.08, 14.44),
    ("BU", "Budapest", "HU", 47.50, 19.04),
    // Eastern Europe
    ("BU", "Bucharest", "RO", 44.43, 26.10),
    ("SO", "Sofia", "BG", 42.70, 23.32),
    ("KI", "Kyiv", "UA", 50.45, 30.52),
    ("BE", "Belgrade", "RS", 44.79, 20.47),
    ("ZA", "Zagreb", "HR", 45.81, 15.98),
    ("LJ", "Ljubljana", "SI", 46.06, 14.51),
    ("TA", "Tallinn", "EE", 59.44, 24.75),
    ("RI", "Riga", "LV", 56.95, 24.11),
    ("VI", "Vilnius", "LT", 54.69, 25.28),
    // Southern Europe
    ("AT", "Athens", "GR", 37.98, 23.73),
    ("IS", "Istanbul", "TR", 41.01, 28.98),
    ("NI", "Nicosia", "CY", 35.17, 33.37),
    // Ireland / Iceland
    ("DU", "Dublin", "IE", 53.35, -6.26),
    ("RE", "Reykjavik", "IS", 64.15, -21.94),
    ("LU", "Luxembourg", "LU", 49.61, 6.13),
    // Asia
    ("TK", "Tokyo", "JP", 35.68, 139.69),
    ("OS", "Osaka", "JP", 34.69, 135.50),
    ("SG", "Singapore", "SG", 1.35, 103.82),
    ("HK", "Hong Kong", "HK", 22.32, 114.17),
    ("SE", "Seoul", "KR", 37.57, 126.98),
    ("MU", "Mumbai", "IN", 19.08, 72.88),
    ("BA", "Bangkok", "TH", 13.76, 100.50),
    ("TA", "Taipei", "TW", 25.03, 121.57),
    ("KL", "Kuala Lumpur", "MY", 3.14, 101.69),
    ("JA", "Jakarta", "ID", -6.21, 106.85),
    ("MA", "Manila", "PH", 14.60, 120.98),
    ("HA", "Hanoi", "VN", 21.03, 105.85),
    // Oceania
    ("SY", "Sydney", "AU", -33.87, 151.21),
    ("ME", "Melbourne", "AU", -37.81, 144.96),
    ("BR", "Brisbane", "AU", -27.47, 153.03),
    ("PE", "Perth", "AU", -31.95, 115.86),
    ("AU", "Auckland", "NZ", -36.85, 174.76),
    // South America
    ("SP", "São Paulo", "BR", -23.55, -46.63),
    ("RJ", "Rio de Janeiro", "BR", -22.91, -43.17),
    ("BA", "Buenos Aires", "AR", -34.60, -58.38),
    ("SA", "Santiago", "CL", -33.45, -70.67),
    ("BO", "Bogota", "CO", 4.71, -74.07),
    ("LI", "Lima", "PE", -12.05, -77.04),
    ("MX", "Mexico City", "MX", 19.43, -99.13),
    ("SJ", "San Jose", "CR", 9.93, -84.08),
    // Africa
    ("JO", "Johannesburg", "ZA", -26.20, 28.05),
    ("CA", "Cape Town", "ZA", -33.93, 18.42),
    ("CA", "Cairo", "EG", 30.04, 31.24),
    ("LA", "Lagos", "NG", 6.52, 3.38),
    ("NA", "Nairobi", "KE", -1.29, 36.82),
    // Middle East
    ("DU", "Dubai", "AE", 25.20, 55.27),
    ("RI", "Riyadh", "SA", 24.71, 46.68),
    ("TE", "Tel Aviv", "IL", 32.09, 34.78),
    ("DO", "Doha", "QA", 25.29, 51.53),
];

/// Fallback user location used only for great circle arc origin
/// when no user coordinates are configured.
pub const FALLBACK_USER_LAT: f64 = 39.83;
pub const FALLBACK_USER_LON: f64 = -98.58; // Geographic center of the US

/// Look up coordinates by ProtonVPN server name.
///
/// Server names follow patterns like:
///   "US-NY#563"  → country=US, city_code=NY
///   "NL#42"      → country=NL, no city code
///   "JP-TK#12"   → country=JP, city_code=TK
pub fn lookup_server(server_name: &str) -> Option<(f64, f64)> {
    // Try to parse "CC-CITY#N" format
    let base = server_name.split('#').next().unwrap_or(server_name);
    let parts: Vec<&str> = base.split('-').collect();

    match parts.len() {
        2 => {
            let country = parts[0];
            let city_code = parts[1];
            // Match by country + city code
            lookup_city_code(country, city_code)
                .or_else(|| lookup_country(country))
        }
        1 => {
            let country = parts[0];
            lookup_country(country)
        }
        _ => None,
    }
}

/// Look up by country code and city code.
fn lookup_city_code(country: &str, city_code: &str) -> Option<(f64, f64)> {
    let cc = country.to_uppercase();
    let city = city_code.to_uppercase();

    CITIES.iter().find_map(|(code, _, cty, lat, lon)| {
        if code.to_uppercase() == city && cty.to_uppercase() == cc {
            Some((*lat, *lon))
        } else {
            None
        }
    })
}

/// Look up by country code only (returns first/capital city).
fn lookup_country(country: &str) -> Option<(f64, f64)> {
    let cc = country.to_uppercase();

    CITIES.iter().find_map(|(_, _, cty, lat, lon)| {
        if cty.to_uppercase() == cc {
            Some((*lat, *lon))
        } else {
            None
        }
    })
}

/// Look up by city name (fuzzy, case-insensitive).
pub fn lookup_city_name(name: &str) -> Option<(f64, f64)> {
    let lower = name.to_lowercase();

    CITIES.iter().find_map(|(_, city_name, _, lat, lon)| {
        if city_name.to_lowercase().contains(&lower) {
            Some((*lat, *lon))
        } else {
            None
        }
    })
}

/// Get city name from a server name string.
pub fn server_city_name(server_name: &str) -> Option<&'static str> {
    let base = server_name.split('#').next().unwrap_or(server_name);
    let parts: Vec<&str> = base.split('-').collect();

    if parts.len() == 2 {
        let country = parts[0].to_uppercase();
        let city_code = parts[1].to_uppercase();

        CITIES.iter().find_map(|(code, name, cty, _, _)| {
            if code.to_uppercase() == city_code && cty.to_uppercase() == country {
                Some(*name)
            } else {
                None
            }
        })
    } else {
        None
    }
}
