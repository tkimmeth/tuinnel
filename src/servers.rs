// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// servers.rs — Server discovery, metadata, and selection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Scans ~/.config/tuinnel/servers/ for .conf and .ovpn files.
// Directory structure provides convenience metadata:
//   servers/<provider>/<country>/<city>/file.conf
// A servers.toml sidecar can override any inferred value.

use crate::backend::Protocol;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ── ServerEntry ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ServerEntry {
    /// Display name (derived from filename without extension).
    pub name: String,
    /// Absolute path to the .conf or .ovpn file.
    pub config_path: PathBuf,
    /// Protocol inferred from file extension.
    pub protocol: Protocol,
    /// Provider name (from directory or metadata).
    pub provider: String,
    /// Country code, uppercase (e.g. "US").
    pub country: String,
    /// City name (e.g. "New York").
    pub city: String,
    /// Coordinates for globe rendering.
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Freeform tags (e.g. "p2p", "streaming").
    pub tags: Vec<String>,
    /// Lower = higher priority for selection.
    pub priority: Option<u32>,
}

// ── Sidecar metadata ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct SidecarFile {
    #[serde(default)]
    metadata: HashMap<String, SidecarEntry>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct SidecarEntry {
    provider: Option<String>,
    country: Option<String>,
    city: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    tags: Option<Vec<String>>,
    priority: Option<u32>,
}

// ── Discovery ──────────────────────────────────────────────────────────────

/// Scan a directory recursively for .conf and .ovpn files, building
/// ServerEntry structs with metadata inferred from path structure
/// and optionally overridden by servers.toml.
pub fn discover(servers_dir: &Path) -> Vec<ServerEntry> {
    if !servers_dir.is_dir() {
        log::warn!("Servers directory does not exist: {}", servers_dir.display());
        return Vec::new();
    }

    let sidecar = load_sidecar(servers_dir);
    let mut entries = Vec::new();

    walk_dir(servers_dir, servers_dir, &sidecar, &mut entries);

    entries.sort_by(|a, b| {
        a.priority
            .unwrap_or(u32::MAX)
            .cmp(&b.priority.unwrap_or(u32::MAX))
            .then_with(|| a.country.cmp(&b.country))
            .then_with(|| a.city.cmp(&b.city))
            .then_with(|| a.name.cmp(&b.name))
    });

    log::info!("Discovered {} server configs", entries.len());
    entries
}

fn walk_dir(
    base: &Path,
    dir: &Path,
    sidecar: &HashMap<String, SidecarEntry>,
    entries: &mut Vec<ServerEntry>,
) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(base, &path, sidecar, entries);
        } else if let Some(server) = parse_config_file(base, &path, sidecar) {
            entries.push(server);
        }
    }
}

fn parse_config_file(
    base: &Path,
    path: &Path,
    sidecar: &HashMap<String, SidecarEntry>,
) -> Option<ServerEntry> {
    let ext = path.extension()?.to_str()?;
    let protocol = match ext {
        "conf" => Protocol::WireGuard,
        "ovpn" => Protocol::OpenVPN,
        _ => return None,
    };

    let stem = path.file_stem()?.to_str()?.to_string();

    // Infer metadata from directory structure relative to base:
    //   base/<provider>/<country>/<city>/file.conf
    let relative = path.strip_prefix(base).ok()?;
    let components: Vec<&str> = relative
        .parent()?
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    let (provider, country, city) = match components.len() {
        0 => (String::new(), String::new(), String::new()),
        1 => (components[0].to_string(), String::new(), String::new()),
        2 => (
            components[0].to_string(),
            components[1].to_uppercase(),
            String::new(),
        ),
        _ => (
            components[0].to_string(),
            components[1].to_uppercase(),
            components[2].to_string(),
        ),
    };

    // Apply sidecar overrides
    let overrides = sidecar.get(&stem).cloned().unwrap_or_default();

    let entry = ServerEntry {
        name: stem,
        config_path: path.to_path_buf(),
        protocol,
        provider: overrides.provider.unwrap_or(provider),
        country: overrides.country.map(|c| c.to_uppercase()).unwrap_or(country),
        city: overrides.city.unwrap_or(city),
        lat: overrides.lat,
        lon: overrides.lon,
        tags: overrides.tags.unwrap_or_default(),
        priority: overrides.priority,
    };

    Some(entry)
}

fn load_sidecar(servers_dir: &Path) -> HashMap<String, SidecarEntry> {
    let sidecar_path = servers_dir.join("servers.toml");
    if !sidecar_path.exists() {
        return HashMap::new();
    }

    match fs::read_to_string(&sidecar_path) {
        Ok(text) => match toml::from_str::<SidecarFile>(&text) {
            Ok(file) => {
                log::info!("Loaded {} sidecar entries", file.metadata.len());
                file.metadata
            }
            Err(e) => {
                log::warn!("Failed to parse servers.toml: {e}");
                HashMap::new()
            }
        },
        Err(e) => {
            log::warn!("Failed to read servers.toml: {e}");
            HashMap::new()
        }
    }
}

// ── Selection helpers ──────────────────────────────────────────────────────

pub fn find_by_country<'a>(servers: &'a [ServerEntry], code: &str) -> Vec<&'a ServerEntry> {
    let upper = code.to_uppercase();
    servers.iter().filter(|s| s.country == upper).collect()
}

pub fn find_by_city<'a>(servers: &'a [ServerEntry], name: &str) -> Vec<&'a ServerEntry> {
    let lower = name.to_lowercase();
    servers
        .iter()
        .filter(|s| s.city.to_lowercase().contains(&lower))
        .collect()
}

pub fn find_by_name<'a>(servers: &'a [ServerEntry], query: &str) -> Option<&'a ServerEntry> {
    let lower = query.to_lowercase();
    servers.iter().find(|s| s.name.to_lowercase() == lower)
}

/// List unique countries as (code, code) pairs.
/// The second element is the code itself since we don't have full names
/// without a geo lookup — callers can resolve display names via geo.rs.
pub fn countries(servers: &[ServerEntry]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for s in servers {
        if !s.country.is_empty() && seen.insert(s.country.clone()) {
            result.push((s.country.clone(), s.country.clone()));
        }
    }
    result.sort();
    result
}

/// List unique cities for a country code.
pub fn cities(servers: &[ServerEntry], country: &str) -> Vec<String> {
    let upper = country.to_uppercase();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for s in servers {
        if s.country == upper && !s.city.is_empty() && seen.insert(s.city.clone()) {
            result.push(s.city.clone());
        }
    }
    result.sort();
    result
}

pub fn pick_first(servers: &[ServerEntry]) -> Option<&ServerEntry> {
    servers.first()
}

pub fn pick_random(servers: &[ServerEntry]) -> Option<&ServerEntry> {
    if servers.is_empty() {
        return None;
    }
    // Simple random without pulling in rand crate
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    Some(&servers[nanos % servers.len()])
}

use std::time::SystemTime;
