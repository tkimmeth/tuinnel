// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// import.rs — `tuinnel import <source>` — bulk WireGuard config ingestion
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Source forms accepted:
//   - single .conf / .ovpn file
//   - directory of .conf files (walked recursively)
//   - .zip archive (most providers ship configs this way)
//   - https:// URL — downloaded, then dispatched as zip / single config
//
// All entries are validated through `wireguard::validate_wg_config` before
// they are written to disk; any file containing `PostUp`/`PostDown`/`PreUp`/
// `PreDown`/`Table`/`FwMark`/`SaveConfig` is rejected. See ADR-004.
//
// Final layout under the configured servers_dir:
//   <servers_dir>/<provider>/<country>/<city>/<stem>.conf  (mode 0600)

use crate::config::{servers_dir, Config};
use crate::output::OutputBuffer;
use crate::wireguard;

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum size of a single HTTPS download. Provider config bundles are
/// typically a few hundred KB; 10 MB is a generous ceiling.
const MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024;

/// CLI arguments forwarded from `main.rs`.
pub struct ImportArgs {
    pub source: PathBuf,
    pub force: bool,
    pub dry_run: bool,
    pub provider: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
}

#[derive(Debug, Clone)]
struct Metadata {
    provider: String,
    country: String,
    city: String,
}

impl Metadata {
    fn empty() -> Self {
        Self {
            provider: String::new(),
            country: String::new(),
            city: String::new(),
        }
    }
}

/// Top-level entry called from `main.rs::Commands::Import`.
pub fn run(args: ImportArgs, config: &Config) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Import");

    let source_str = args.source.to_string_lossy().into_owned();

    // URL form
    if source_str.starts_with("http://") {
        buf.err("HTTP URLs are not allowed; use https://");
        return buf;
    }
    if source_str.starts_with("https://") {
        match download_to_temp(&source_str) {
            Ok((tmp_dir, downloaded)) => {
                buf.ok(&format!("Downloaded {} ({} bytes)", source_str, file_size(&downloaded)));
                let dispatch_args = ImportArgs {
                    source: downloaded,
                    force: args.force,
                    dry_run: args.dry_run,
                    provider: args.provider.clone(),
                    country: args.country.clone(),
                    city: args.city.clone(),
                };
                let inner = run_local(&dispatch_args, config);
                buf.lines.extend(inner.lines);
                let _ = fs::remove_dir_all(&tmp_dir);
                return buf;
            }
            Err(e) => {
                buf.err(&format!("Download failed: {e}"));
                return buf;
            }
        }
    }

    // Local source
    let inner = run_local(&args, config);
    buf.lines.extend(inner.lines);
    buf
}

fn run_local(args: &ImportArgs, config: &Config) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    if !args.source.exists() {
        buf.err(&format!("Source not found: {}", args.source.display()));
        return buf;
    }

    let dest_root = servers_dir(config);
    if !args.dry_run {
        if let Err(e) = fs::create_dir_all(&dest_root) {
            buf.err(&format!("Cannot create servers dir {}: {e}", dest_root.display()));
            return buf;
        }
    }

    let cli_meta = Metadata {
        provider: args.provider.clone().unwrap_or_default(),
        country: args.country.clone().unwrap_or_default().to_uppercase(),
        city: args.city.clone().unwrap_or_default(),
    };

    if args.source.is_file() {
        let ext = args
            .source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "zip" => import_zip(&args.source, &dest_root, &cli_meta, args, &mut buf),
            "conf" | "ovpn" => {
                let n = import_one(&args.source, &dest_root, &cli_meta, args, &mut buf);
                buf.blank();
                buf.dim(&format!("Imported {n} config(s)"));
            }
            _ => {
                buf.err(&format!(
                    "Unsupported file type `.{ext}` — expected .conf, .ovpn, or .zip"
                ));
            }
        }
    } else if args.source.is_dir() {
        let mut total = 0usize;
        walk_and_import(&args.source, &dest_root, &cli_meta, args, &mut buf, &mut total);
        buf.blank();
        buf.dim(&format!("Imported {total} config(s)"));
    } else {
        buf.err(&format!("Source is neither file nor directory: {}", args.source.display()));
    }

    buf
}

fn walk_and_import(
    dir: &Path,
    dest_root: &Path,
    cli_meta: &Metadata,
    args: &ImportArgs,
    buf: &mut OutputBuffer,
    total: &mut usize,
) {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            buf.err(&format!("Cannot read {}: {e}", dir.display()));
            return;
        }
    };

    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_and_import(&path, dest_root, cli_meta, args, buf, total);
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "conf" && ext != "ovpn" {
            continue;
        }

        *total += import_one(&path, dest_root, cli_meta, args, buf);
    }
}

fn import_zip(
    zip_path: &Path,
    dest_root: &Path,
    cli_meta: &Metadata,
    args: &ImportArgs,
    buf: &mut OutputBuffer,
) {
    let file = match fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            buf.err(&format!("Cannot open {}: {e}", zip_path.display()));
            return;
        }
    };

    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            buf.err(&format!("Not a valid zip: {e}"));
            return;
        }
    };

    let tmp_dir = match make_temp_dir("tuinnel-import") {
        Ok(d) => d,
        Err(e) => {
            buf.err(&format!("Cannot create temp dir: {e}"));
            return;
        }
    };

    // Extract every .conf / .ovpn entry into the temp dir, ignoring
    // archive-internal directories (we re-derive structure from filenames).
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                buf.warn(&format!("Skipping zip entry {i}: {e}"));
                continue;
            }
        };

        let name = entry.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        // Reject zip-slip: refuse absolute paths or `..` segments.
        if name.starts_with('/') || name.split('/').any(|c| c == "..") {
            buf.warn(&format!("Skipping suspicious zip entry: {name}"));
            continue;
        }

        let basename = match Path::new(&name).file_name().and_then(|s| s.to_str()) {
            Some(b) => b.to_string(),
            None => continue,
        };

        let lower = basename.to_lowercase();
        if !(lower.ends_with(".conf") || lower.ends_with(".ovpn")) {
            continue;
        }

        let out_path = tmp_dir.join(&basename);
        let mut out = match fs::File::create(&out_path) {
            Ok(f) => f,
            Err(e) => {
                buf.warn(&format!("Cannot extract {name}: {e}"));
                continue;
            }
        };
        if let Err(e) = std::io::copy(&mut entry, &mut out) {
            buf.warn(&format!("Cannot extract {name}: {e}"));
            continue;
        }
    }

    let mut total = 0usize;
    walk_and_import(&tmp_dir, dest_root, cli_meta, args, buf, &mut total);

    let _ = fs::remove_dir_all(&tmp_dir);

    buf.blank();
    buf.dim(&format!("Imported {total} config(s) from {}", zip_path.display()));
}

/// Returns 1 if a config was imported (or would be in dry-run), 0 otherwise.
fn import_one(
    src: &Path,
    dest_root: &Path,
    cli_meta: &Metadata,
    args: &ImportArgs,
    buf: &mut OutputBuffer,
) -> usize {
    // Default to `.conf` if the source has no extension — most providers
    // ship plain WireGuard configs.
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("conf")
        .to_lowercase();

    // Validate WireGuard configs first — refuse to ingest one that wg-quick
    // would treat as a shell snippet. ADR-004. OpenVPN has its own dangerous
    // directives (script-security, up, down) but its backend isn't wired
    // yet (ADR-001 says v1 is WG-only); we accept them as-is for now.
    if ext == "conf" {
        if let Err(e) = wireguard::validate_wg_config(src) {
            buf.err(&format!("Rejected {}: {}", src.display(), e));
            return 0;
        }
    }

    let stem = match src.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s.to_string(),
        None => {
            buf.err(&format!("Skipped {}: cannot derive stem", src.display()));
            return 0;
        }
    };

    let inferred = infer_metadata(&stem);
    let meta = merge_metadata(cli_meta, &inferred);

    // Default for any still-empty field — keeps imported configs discoverable
    // even when we have no provider hints at all.
    let provider = if meta.provider.is_empty() { "imported".into() } else { meta.provider };
    let country = meta.country;
    let city = meta.city;

    // Build destination path. Empty country/city collapse the layout, which
    // matches `servers.rs::parse_config_file` inference rules.
    let mut dest = dest_root.join(&provider);
    if !country.is_empty() {
        dest.push(&country);
        if !city.is_empty() {
            dest.push(&city);
        }
    }

    let final_path = dest.join(format!("{stem}.{ext}"));

    if final_path.exists() && !args.force {
        buf.warn(&format!(
            "Skipped (exists, use --force): {}",
            final_path.display()
        ));
        return 0;
    }

    if args.dry_run {
        buf.kv(&stem, &format!("→ {} [{}]", final_path.display(), describe_meta(&provider, &country, &city)));
        return 1;
    }

    if let Err(e) = fs::create_dir_all(&dest) {
        buf.err(&format!("Cannot create {}: {e}", dest.display()));
        return 0;
    }

    if let Err(e) = copy_secret(src, &final_path) {
        buf.err(&format!("Copy failed: {e}"));
        return 0;
    }

    buf.ok(&format!("Imported {stem} → {}", final_path.display()));
    1
}

fn describe_meta(provider: &str, country: &str, city: &str) -> String {
    match (country.is_empty(), city.is_empty()) {
        (true, true) => format!("provider={provider}"),
        (false, true) => format!("provider={provider}, country={country}"),
        (false, false) => format!("provider={provider}, country={country}, city={city}"),
        (true, false) => format!("provider={provider}, city={city}"),
    }
}

fn merge_metadata(cli: &Metadata, inferred: &Metadata) -> Metadata {
    Metadata {
        provider: if !cli.provider.is_empty() { cli.provider.clone() } else { inferred.provider.clone() },
        country: if !cli.country.is_empty() { cli.country.clone() } else { inferred.country.clone() },
        city: if !cli.city.is_empty() { cli.city.clone() } else { inferred.city.clone() },
    }
}

/// Best-effort metadata inference from filename stems. Conservative — when
/// a heuristic doesn't match, return an empty field rather than guessing
/// (a wrong country code is worse than a missing one).
fn infer_metadata(stem: &str) -> Metadata {
    let lower = stem.to_lowercase();

    // Mullvad: "mullvad-XX-name-N"
    if let Some(rest) = lower.strip_prefix("mullvad-") {
        let parts: Vec<&str> = rest.split('-').collect();
        if let Some(cc) = parts.first() {
            if cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic()) {
                return Metadata {
                    provider: "mullvad".into(),
                    country: cc.to_uppercase(),
                    city: String::new(),
                };
            }
        }
    }

    // ProtonVPN: "XX-NAME#N" or "XX-CITY-NN"
    if let Some(idx) = stem.find('#') {
        let head = &stem[..idx];
        if let Some((cc, _)) = head.split_once('-') {
            if cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic()) {
                return Metadata {
                    provider: "protonvpn".into(),
                    country: cc.to_uppercase(),
                    city: String::new(),
                };
            }
        }
    }

    // ProtonVPN WireGuard: "wg-XX-N" (the convention used by Proton's portal
    // download for WireGuard, e.g. wg-NL-1.conf, wg-FI-14.conf). Without
    // this match the generic case below would grab "wg" as the country code.
    if let Some(rest) = lower.strip_prefix("wg-") {
        if let Some((cc, _)) = rest.split_once('-') {
            if cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic()) {
                return Metadata {
                    provider: "protonvpn".into(),
                    country: cc.to_uppercase(),
                    city: String::new(),
                };
            }
        }
    }

    // Generic "XX-something" (likely WireGuard stem with country prefix)
    if let Some((cc, _)) = stem.split_once('-') {
        if cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic()) {
            return Metadata {
                provider: String::new(),
                country: cc.to_uppercase(),
                city: String::new(),
            };
        }
    }

    Metadata::empty()
}

/// Copy a config file to `dest`, ensuring the result has mode 0600. Configs
/// embed WireGuard private keys, so the default umask result (0644) is
/// world-readable and unacceptable.
fn copy_secret(src: &Path, dest: &Path) -> Result<(), String> {
    let content = fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    let mut out = opts
        .open(dest)
        .map_err(|e| format!("open {}: {e}", dest.display()))?;
    out.write_all(&content)
        .map_err(|e| format!("write {}: {e}", dest.display()))?;
    // mode() in OpenOptions only applies on create; enforce on overwrite too.
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dest, fs::Permissions::from_mode(0o600));
    Ok(())
}

fn file_size(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

// ── HTTPS download ─────────────────────────────────────────────────────────

fn download_to_temp(url: &str) -> Result<(PathBuf, PathBuf), String> {
    let tmp_dir = make_temp_dir("tuinnel-import-dl")?;

    // Filename derived from the URL path; default to `download.zip`.
    let basename = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("download.zip")
        .split('?')
        .next()
        .unwrap_or("download.zip");

    let dest = tmp_dir.join(basename);

    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("HTTPS error: {e}"))?;

    let mut reader = resp.into_reader().take(MAX_DOWNLOAD_BYTES + 1);

    let mut out = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&dest)
        .map_err(|e| format!("open {}: {e}", dest.display()))?;

    let mut buf = [0u8; 16 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > MAX_DOWNLOAD_BYTES {
            let _ = fs::remove_file(&dest);
            return Err(format!(
                "Download exceeded {} bytes; refusing",
                MAX_DOWNLOAD_BYTES
            ));
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("write: {e}"))?;
    }

    Ok((tmp_dir, dest))
}

// ── Tiny ad-hoc temp directory helper ──────────────────────────────────────
//
// We avoid pulling in `tempfile` for one call site. The directory is created
// with 0700 inside `$TMPDIR` (or `/tmp`), with a name that includes process
// id and nanosecond clock — collision probability is negligible for
// short-lived imports.

fn make_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let name = format!("{prefix}-{pid}-{nanos}");
    let path = base.join(name);

    fs::create_dir(&path).map_err(|e| format!("mkdir {}: {e}", path.display()))?;

    // Set mode after creation; create_dir respects umask, which may have
    // left group/other readable.
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o700));

    Ok(path)
}
