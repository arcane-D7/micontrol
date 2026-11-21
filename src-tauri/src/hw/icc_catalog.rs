//! Curated ICC catalogue: validation, download and staged installation.
//!
//! The catalogue is embedded at compile time (`src-tauri/data/icc-catalog.json`)
//! so the app never depends on a remote index. Every rule here exists to keep a
//! wrong file away from the display:
//!
//! - a profile must be a **display** profile (`mntr`) in an **RGB** colour
//!   space, with the declared size equal to the real size and the `acsp`
//!   signature at offset 36;
//! - downloaded profiles must match a **pinned** sha256 and byte count from the
//!   manifest — the ICC registry also serves HTML with an ICC content type, so
//!   "the download succeeded" proves nothing;
//! - installation only accepts file names from the catalogue, reads from the
//!   fixed staging directory, and re-validates the bytes as the elevated user
//!   before replacing anything in the system colour directory.
//!
//! Nothing here applies a profile automatically: download and install are two
//! explicit steps, and the display is only touched by the caller
//! (`color_calibration::load_icc_profile`).

use crate::hw::panel_edid::MonitorIdentity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Hard ceiling for a profile file (real display profiles are a few KiB; the
/// ICC spec allows more, the app does not need it).
pub const MAX_ICC_BYTES: u64 = 32 * 1024 * 1024;

const CATALOG_JSON: &str = include_str!("../../data/icc-catalog.json");

// ── Manifest types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IccKind {
    /// Already present in `%SystemRoot%\System32\spool\drivers\color`.
    Local,
    /// Fetched from `source_url`, verified against the pinned hash, staged,
    /// then installed by the elevated helper.
    Download,
}

/// Which panels a catalogue entry is meant for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PanelMatch {
    /// Colour-space baseline, valid for any display.
    Any,
    /// Only for specific EDID panels.
    Panel {
        #[serde(default)]
        manufacturer: Option<String>,
        #[serde(default)]
        product_codes: Vec<String>,
        #[serde(default)]
        friendly_name_contains: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IccCatalogEntry {
    pub id: String,
    pub display_name: String,
    /// File name in the system colour directory.
    pub profile_name: String,
    /// File name used when installing a download; falls back to `profile_name`.
    #[serde(default)]
    pub install_name: String,
    pub kind: IccKind,
    pub source: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub license_note: String,
    /// Enforced for downloads; `None` for files that ship with Windows/OEM.
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    /// Hash observed on the reference machine — informational only, never
    /// enforced (Windows builds ship different bytes for the same file name).
    #[serde(default)]
    pub sha256_reference: Option<String>,
    #[serde(default)]
    pub bytes_reference: Option<u64>,
    #[serde(default)]
    pub color_space: String,
    #[serde(default)]
    pub description: String,
    pub panel_match: PanelMatch,
}

impl IccCatalogEntry {
    /// File name this entry installs or references.
    pub fn file_name(&self) -> &str {
        if self.kind == IccKind::Download && !self.install_name.is_empty() {
            &self.install_name
        } else {
            &self.profile_name
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IccCatalog {
    pub version: u32,
    pub updated: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub entries: Vec<IccCatalogEntry>,
}

/// The bundled catalogue. Parsing is done once; a malformed bundled file is a
/// build defect, so callers get a plain error instead of an empty list.
pub fn catalog() -> Result<&'static IccCatalog, String> {
    static CACHE: OnceLock<Result<IccCatalog, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            serde_json::from_str::<IccCatalog>(CATALOG_JSON)
                .map_err(|e| format!("bundled ICC catalogue is invalid: {e}"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

pub fn find_entry(id: &str) -> Result<&'static IccCatalogEntry, String> {
    catalog()?
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("unknown ICC catalogue entry: {id}"))
}

// ── ICC validation ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccError {
    TooShort { actual: usize },
    TooLarge { actual: u64 },
    SizeMismatch { declared: u32, actual: usize },
    NotIcc,
    WrongClass { class: String },
    WrongColorSpace { space: String },
    HashMismatch { expected: String, actual: String },
    SizePinMismatch { expected: u64, actual: u64 },
    Io { detail: String },
}

impl std::fmt::Display for IccError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { actual } => write!(
                f,
                "file is {actual} bytes — smaller than the 128-byte ICC header"
            ),
            Self::TooLarge { actual } => write!(
                f,
                "file is {actual} bytes — above the {} MiB limit",
                MAX_ICC_BYTES / (1024 * 1024)
            ),
            Self::SizeMismatch { declared, actual } => write!(
                f,
                "ICC header declares {declared} bytes but the file has {actual}"
            ),
            Self::NotIcc => write!(f, "not an ICC profile: missing the 'acsp' signature"),
            Self::WrongClass { class } => write!(
                f,
                "profile class '{class}' is not a display profile ('mntr')"
            ),
            Self::WrongColorSpace { space } => write!(
                f,
                "colour space '{space}' is not RGB — not a monitor profile"
            ),
            Self::HashMismatch { expected, actual } => {
                write!(f, "sha256 mismatch: expected {expected}, got {actual}")
            }
            Self::SizePinMismatch { expected, actual } => write!(
                f,
                "size mismatch: the catalogue pins {expected} bytes, the file has {actual}"
            ),
            Self::Io { detail } => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for IccError {}

impl From<String> for IccError {
    fn from(detail: String) -> Self {
        Self::Io { detail }
    }
}

/// Facts read from a validated ICC header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IccFileInfo {
    pub bytes: u64,
    pub declared_size: u32,
    pub profile_class: String,
    pub color_space: String,
    pub pcs: String,
    /// `4.3.0` style, from the header's version byte.
    pub version: String,
    pub sha256: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Validate the ICC header of `bytes` and hash it.
///
/// Deliberately strict: a display profile that Windows would silently ignore is
/// an error the user can act on, not a success.
pub fn validate_icc(bytes: &[u8]) -> Result<IccFileInfo, IccError> {
    let actual = bytes.len();
    if actual < 128 {
        return Err(IccError::TooShort { actual });
    }
    if actual as u64 > MAX_ICC_BYTES {
        return Err(IccError::TooLarge {
            actual: actual as u64,
        });
    }
    let declared = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if declared as usize != actual {
        return Err(IccError::SizeMismatch { declared, actual });
    }
    if &bytes[36..40] != b"acsp" {
        return Err(IccError::NotIcc);
    }
    let profile_class = String::from_utf8_lossy(&bytes[12..16]).to_string();
    if profile_class != "mntr" {
        return Err(IccError::WrongClass {
            class: profile_class.trim().to_string(),
        });
    }
    let color_space = String::from_utf8_lossy(&bytes[16..20]).to_string();
    if color_space != "RGB " {
        return Err(IccError::WrongColorSpace {
            space: color_space.trim().to_string(),
        });
    }
    Ok(IccFileInfo {
        bytes: actual as u64,
        declared_size: declared,
        profile_class: profile_class.trim().to_string(),
        color_space: color_space.trim().to_string(),
        pcs: String::from_utf8_lossy(&bytes[20..24]).trim().to_string(),
        version: format!("{}.{}.{}", bytes[8], bytes[9] >> 4, bytes[9] & 0x0f),
        sha256: sha256_hex(bytes),
    })
}

// ── Paths ────────────────────────────────────────────────────────────────────

/// `%SystemRoot%\System32\spool\drivers\color`.
pub fn system_profiles_dir() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root)
        .join("System32")
        .join("spool")
        .join("drivers")
        .join("color")
}

/// Fixed staging directory for downloaded profiles. The elevated helper reads
/// from here and nowhere else.
pub fn downloads_dir() -> PathBuf {
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(base)
        .join("MiControl")
        .join("icc")
        .join("downloads")
}

/// Validate a file name that will cross the privilege boundary.
///
/// `[A-Za-z0-9._-]{1,64}`, no leading dot, no `..`, extension `.icc`/`.icm`.
pub fn sanitize_profile_file_name(name: &str) -> Result<String, String> {
    if name.is_empty() || name.len() > 64 {
        return Err("profile file name must be 1-64 characters".to_string());
    }
    if name.starts_with('.') {
        return Err("profile file name must not start with a dot".to_string());
    }
    if name.contains("..") {
        return Err("profile file name must not contain '..'".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!(
            "profile file name contains unsupported characters: {name}"
        ));
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if !matches!(extension.as_deref(), Some("icc") | Some("icm")) {
        return Err("profile file name must end in .icc or .icm".to_string());
    }
    Ok(name.to_string())
}

// ── Matching against the detected panel ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchQuality {
    /// The entry names this panel's EDID manufacturer + product code.
    Exact,
    /// A colour-space baseline that applies to any display.
    Generic,
}

fn normalize_hex_code(code: &str) -> String {
    code.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_ascii_uppercase()
}

/// `None` = the entry is not meant for this panel (hide it).
pub fn match_quality(
    entry: &IccCatalogEntry,
    monitor: Option<&MonitorIdentity>,
) -> Option<MatchQuality> {
    match &entry.panel_match {
        PanelMatch::Any => Some(MatchQuality::Generic),
        PanelMatch::Panel {
            manufacturer,
            product_codes,
            friendly_name_contains,
        } => {
            let monitor = monitor?;
            if let Some(mfg) = manufacturer {
                if !mfg.eq_ignore_ascii_case(&monitor.manufacturer) {
                    return None;
                }
            }
            if product_codes.is_empty() && friendly_name_contains.is_empty() {
                // Manufacturer-wide entry: no product code and no name filter.
                return if manufacturer.is_some() {
                    Some(MatchQuality::Exact)
                } else {
                    None
                };
            }
            let product_ok = product_codes
                .iter()
                .any(|code| normalize_hex_code(code) == monitor.product_code);
            let name_ok = !monitor.friendly_name.is_empty()
                && friendly_name_contains.iter().any(|needle| {
                    monitor
                        .friendly_name
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
                });
            if product_ok || name_ok {
                Some(MatchQuality::Exact)
            } else {
                None
            }
        }
    }
}

/// One catalogue row as the colour tab needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntryView {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub source: String,
    pub source_url: Option<String>,
    pub license_note: String,
    pub kind: IccKind,
    pub color_space: String,
    pub sha256: Option<String>,
    pub sha256_reference: Option<String>,
    pub bytes: Option<u64>,
    pub file_name: String,
    /// The profile file is already present in the system colour directory.
    pub available_locally: bool,
    /// `exact` (panel-specific) or `generic` (any panel).
    pub match_quality: MatchQuality,
    pub compatible: bool,
}

/// Entries that apply to `monitor`, panel-specific ones first.
///
/// When the panel is unknown only `Any` entries are returned — a panel-specific
/// profile on an unidentified panel is exactly the guess this feature refuses
/// to make.
pub fn entries_for(monitor: Option<&MonitorIdentity>, system_dir: &Path) -> Vec<CatalogEntryView> {
    let Ok(catalog) = catalog() else {
        return Vec::new();
    };
    let mut out: Vec<CatalogEntryView> = catalog
        .entries
        .iter()
        .filter_map(|entry| {
            let quality = match_quality(entry, monitor)?;
            Some(CatalogEntryView {
                id: entry.id.clone(),
                display_name: entry.display_name.clone(),
                description: entry.description.clone(),
                source: entry.source.clone(),
                source_url: entry.source_url.clone(),
                license_note: entry.license_note.clone(),
                kind: entry.kind,
                color_space: entry.color_space.clone(),
                sha256: entry.sha256.clone(),
                sha256_reference: entry.sha256_reference.clone(),
                bytes: entry.bytes.or(entry.bytes_reference),
                file_name: entry.file_name().to_string(),
                available_locally: system_dir.join(entry.file_name()).is_file(),
                match_quality: quality,
                compatible: quality == MatchQuality::Exact,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.compatible
            .cmp(&a.compatible)
            .then_with(|| a.kind_is_download().cmp(&b.kind_is_download()))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    out
}

impl CatalogEntryView {
    fn kind_is_download(&self) -> bool {
        matches!(self.kind, IccKind::Download)
    }
}

// ── Download ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub entry_id: String,
    pub file_name: String,
    pub staging_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub profile_class: String,
    pub color_space: String,
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(20))
        .user_agent(concat!(
            "MiControl/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/arcane-D7/micontrol; ICC catalogue)"
        ))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

/// Download a catalogue entry, validate it against the pinned sha256/bytes and
/// stage it. The display is not touched here.
pub async fn download_entry(entry_id: &str) -> Result<DownloadResult, String> {
    let entry = find_entry(entry_id)?;
    if entry.kind != IccKind::Download {
        return Err(format!(
            "'{}' is a local profile entry — apply it instead of downloading",
            entry.display_name
        ));
    }
    let expected_sha = entry
        .sha256
        .as_deref()
        .ok_or_else(|| format!("catalogue entry '{}' has no pinned sha256", entry.id))?
        .to_ascii_lowercase();
    if expected_sha.len() != 64 || !expected_sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "catalogue entry '{}' has an invalid sha256 pin",
            entry.id
        ));
    }
    let expected_bytes = entry
        .bytes
        .ok_or_else(|| format!("catalogue entry '{}' has no pinned size", entry.id))?;
    if expected_bytes == 0 || expected_bytes > MAX_ICC_BYTES {
        return Err(format!(
            "catalogue entry '{}' pins an unusable size ({expected_bytes} bytes)",
            entry.id
        ));
    }
    let url = entry
        .source_url
        .as_deref()
        .ok_or_else(|| format!("catalogue entry '{}' has no download URL", entry.id))?;
    if !url.starts_with("https://") {
        return Err("profile downloads must use https".to_string());
    }

    let file_name = sanitize_profile_file_name(entry.file_name())?;
    let client = http_client()?;
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "application/vnd.iccprofile, application/octet-stream",
        )
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("GET {url}: HTTP {status}"));
    }
    if let Some(len) = response.content_length() {
        if len > MAX_ICC_BYTES {
            return Err(format!(
                "GET {url}: server announces {len} bytes, above the 32 MiB limit"
            ));
        }
    }

    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut body: Vec<u8> = Vec::with_capacity(expected_bytes as usize);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("reading {url}: {e}"))?;
        if body.len() as u64 + chunk.len() as u64 > MAX_ICC_BYTES {
            return Err(format!("GET {url}: response exceeded the 32 MiB limit"));
        }
        body.extend_from_slice(&chunk);
    }

    if body.len() as u64 != expected_bytes {
        return Err(IccError::SizePinMismatch {
            expected: expected_bytes,
            actual: body.len() as u64,
        }
        .to_string());
    }
    let info = validate_icc(&body).map_err(|e| e.to_string())?;
    if info.sha256 != expected_sha {
        return Err(IccError::HashMismatch {
            expected: expected_sha,
            actual: info.sha256,
        }
        .to_string());
    }

    let dir = downloads_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(&file_name);
    let partial = dir.join(format!(".{file_name}.part"));
    std::fs::write(&partial, &body)
        .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("cannot replace {}: {e}", path.display()))?;
    }
    std::fs::rename(&partial, &path)
        .map_err(|e| format!("cannot stage {}: {e}", path.display()))?;

    log::info!(
        "[icc] staged {} ({} bytes, sha256 {}) at {}",
        entry.id,
        info.bytes,
        info.sha256,
        path.display()
    );

    Ok(DownloadResult {
        entry_id: entry.id.clone(),
        file_name,
        staging_path: path.display().to_string(),
        sha256: info.sha256,
        bytes: info.bytes,
        profile_class: info.profile_class,
        color_space: info.color_space,
    })
}

// ── Installation (runs as the elevated helper) ──────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallResult {
    pub file_name: String,
    pub installed_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub replaced: bool,
}

/// Read a local profile from the system directory and validate its header.
pub fn validate_local_profile(path: &Path) -> Result<IccFileInfo, String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if metadata.len() > MAX_ICC_BYTES {
        return Err(IccError::TooLarge {
            actual: metadata.len(),
        }
        .to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    validate_icc(&bytes).map_err(|e| e.to_string())
}

/// Copy a staged, catalogued profile into the system colour directory.
///
/// Called **only** from the elevated dispatcher. The file name must belong to a
/// `download` entry in the bundled catalogue; the bytes are re-validated and the
/// pinned hash re-checked here, so a compromised caller cannot install an
/// arbitrary profile by swapping the staging file.
pub fn install_staged_profile(file_name: &str) -> Result<InstallResult, String> {
    let file_name = sanitize_profile_file_name(file_name)?;
    let catalog = catalog()?;
    let entry = catalog
        .entries
        .iter()
        .find(|e| e.kind == IccKind::Download && e.file_name().eq_ignore_ascii_case(&file_name))
        .ok_or_else(|| {
            format!("'{file_name}' is not an installable profile in the bundled catalogue")
        })?;
    let expected_sha = entry
        .sha256
        .as_deref()
        .ok_or_else(|| format!("catalogue entry '{}' has no pinned sha256", entry.id))?
        .to_ascii_lowercase();
    let expected_bytes = entry
        .bytes
        .ok_or_else(|| format!("catalogue entry '{}' has no pinned size", entry.id))?;

    let staging_dir = downloads_dir();
    let staging = staging_dir.join(&file_name);
    let canonical_staging = std::fs::canonicalize(&staging)
        .map_err(|e| format!("staged profile not found at {}: {e}", staging.display()))?;
    let canonical_dir = std::fs::canonicalize(&staging_dir).map_err(|e| {
        format!(
            "staging directory unavailable ({}): {e}",
            staging_dir.display()
        )
    })?;
    if canonical_staging.parent() != Some(canonical_dir.as_path()) {
        return Err(format!(
            "refusing to install {}: not directly inside the staging directory",
            canonical_staging.display()
        ));
    }

    let metadata = std::fs::metadata(&canonical_staging)
        .map_err(|e| format!("cannot stat {}: {e}", canonical_staging.display()))?;
    if metadata.len() > MAX_ICC_BYTES {
        return Err(IccError::TooLarge {
            actual: metadata.len(),
        }
        .to_string());
    }
    if metadata.len() != expected_bytes {
        return Err(IccError::SizePinMismatch {
            expected: expected_bytes,
            actual: metadata.len(),
        }
        .to_string());
    }
    let bytes = std::fs::read(&canonical_staging)
        .map_err(|e| format!("cannot read {}: {e}", canonical_staging.display()))?;
    let info = validate_icc(&bytes).map_err(|e| e.to_string())?;
    if info.sha256 != expected_sha {
        return Err(IccError::HashMismatch {
            expected: expected_sha,
            actual: info.sha256,
        }
        .to_string());
    }

    let target_dir = system_profiles_dir();
    if !target_dir.is_dir() {
        return Err(format!(
            "system colour directory not found: {}",
            target_dir.display()
        ));
    }
    let target = target_dir.join(&file_name);
    let replaced = target.exists();

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let temp = target_dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp)
                .map_err(|e| format!("cannot create {}: {e}", temp.display()))?;
            file.write_all(&bytes)
                .map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
            file.sync_all()
                .map_err(|e| format!("cannot flush {}: {e}", temp.display()))?;
        }

        let wide = |p: &Path| -> Vec<u16> {
            p.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };
        let from = wide(&temp);
        let to = wide(&target);
        let moved = unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if let Err(e) = moved {
            let _ = std::fs::remove_file(&temp);
            return Err(format!(
                "cannot install {}: {e} (close Colour Management and revert the display to sRGB, then retry)",
                target.display()
            ));
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::write(&target, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    }

    // Read back what actually landed: a successful MoveFileEx is not proof the
    // right bytes are on disk.
    let installed =
        std::fs::read(&target).map_err(|e| format!("cannot verify {}: {e}", target.display()))?;
    let installed_info = validate_icc(&installed).map_err(|e| e.to_string())?;
    if installed_info.sha256 != expected_sha {
        return Err(format!(
            "post-install verification failed: {} has sha256 {}, expected {}",
            target.display(),
            installed_info.sha256,
            expected_sha
        ));
    }

    log::info!(
        "[icc] installed {} ({} bytes) at {} (replaced={replaced})",
        entry.id,
        installed_info.bytes,
        target.display()
    );

    Ok(InstallResult {
        file_name,
        installed_path: target.display().to_string(),
        sha256: installed_info.sha256,
        bytes: installed_info.bytes,
        replaced,
    })
}

/// True when the profile is already installed in `system_dir` and its bytes
/// still match the catalogue pin. Lets the UI offer *Apply* instead of forcing a
/// re-download every session.
pub fn installed_profile_matches(entry: &IccCatalogEntry, system_dir: &Path) -> bool {
    let Some(expected) = entry.sha256.as_deref() else {
        return false;
    };
    if expected.len() != 64 {
        return false;
    }
    let path = system_dir.join(entry.file_name());
    match validate_local_profile(&path) {
        Ok(info) => {
            info.sha256.eq_ignore_ascii_case(expected)
                && entry.bytes.is_none_or(|bytes| bytes == info.bytes)
        }
        Err(_) => false,
    }
}

// ── Online research links ───────────────────────────────────────────────────
//
// There is no public database of per-unit OLED calibrations, so the honest
// "search online" action is a real search on the two sites that actually
// document panels. URLs are built here from a fixed template; the query is
// restricted to characters that need no escaping, so nothing user-controlled
// reaches the shell.

/// Build the URL for a panel-research search. `kind` is `notebookcheck` or
/// `panelook`.
pub fn profile_search_url(kind: &str, query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 80 {
        return Err("search query must be 1-80 characters".to_string());
    }
    if !query
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
    {
        return Err("search query contains unsupported characters".to_string());
    }
    let encoded = query.replace(' ', "%20");
    match kind {
        // NotebookCheck's site search is a Google custom-search page; the
        // old `search.html?q=` template answers HTTP 404 (verified in a real
        // browser), so searches never landed anywhere.
        "notebookcheck" => Ok(format!(
            "https://www.notebookcheck.net/Google-Search.36690.0.html?q={encoded}"
        )),
        "panelook" => Ok(format!(
            "https://www.panelook.com/modelsearch.php?keyword={encoded}"
        )),
        other => Err(format!("unknown search provider: {other}")),
    }
}

/// Open a research search in the default browser through `ShellExecuteW`:
/// the shell resolves the browser and no console window is ever created
/// (owner rule B7); the URL is passed as one argument, never to a shell.
pub fn open_profile_search(kind: &str, query: &str) -> Result<(), String> {
    let url = profile_search_url(kind, query)?;
    crate::util::shell_open::open_url(&url).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal structurally valid display profile: 128-byte header + empty tag
    /// table. Enough for every check `validate_icc` performs.
    fn icc_fixture(class: &[u8; 4], space: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 132];
        bytes[0..4].copy_from_slice(&132u32.to_be_bytes());
        bytes[8] = 4;
        bytes[9] = 0x30;
        bytes[12..16].copy_from_slice(class);
        bytes[16..20].copy_from_slice(space);
        bytes[20..24].copy_from_slice(b"XYZ ");
        bytes[36..40].copy_from_slice(b"acsp");
        bytes[128..132].copy_from_slice(&0u32.to_be_bytes());
        bytes
    }

    fn monitor(mfg: &str, code: &str, name: &str) -> MonitorIdentity {
        MonitorIdentity {
            instance_name: format!(r"DISPLAY\{mfg}{code}\4&x_0"),
            manufacturer: mfg.to_string(),
            product_code: code.to_string(),
            serial: "0".to_string(),
            friendly_name: name.to_string(),
            year: None,
            week: None,
        }
    }

    #[test]
    fn accepts_a_minimal_display_profile() {
        let info = validate_icc(&icc_fixture(b"mntr", b"RGB ")).expect("valid fixture");
        assert_eq!(info.bytes, 132);
        assert_eq!(info.profile_class, "mntr");
        assert_eq!(info.color_space, "RGB");
        assert_eq!(info.version, "4.3.0");
        assert_eq!(info.sha256.len(), 64);
    }

    #[test]
    fn rejects_a_size_that_does_not_match_the_header() {
        let mut bytes = icc_fixture(b"mntr", b"RGB ");
        bytes.push(0);
        assert_eq!(
            validate_icc(&bytes),
            Err(IccError::SizeMismatch {
                declared: 132,
                actual: 133
            })
        );
    }

    #[test]
    fn rejects_html_served_with_an_icc_content_type() {
        // The ICC registry returns an HTML error page with
        // `Content-Type: application/vnd.iccprofile`; only the header can tell.
        let page = b"<!DOCTYPE html><html><body>404</body></html>";
        assert!(matches!(validate_icc(page), Err(IccError::TooShort { .. })));
        let mut padded = vec![0u8; 132];
        padded[..page.len()].copy_from_slice(page);
        // A real size in the header, but no 'acsp' signature: the only check
        // that catches an HTML error page served as an ICC profile.
        padded[0..4].copy_from_slice(&132u32.to_be_bytes());
        assert_eq!(validate_icc(&padded), Err(IccError::NotIcc));
    }

    #[test]
    fn rejects_printer_and_colour_space_profiles() {
        assert!(matches!(
            validate_icc(&icc_fixture(b"prtr", b"CMYK")),
            Err(IccError::WrongClass { .. })
        ));
        // Real case: sRGB_v4_ICC_preference.icc is class 'spac' (a colour space).
        assert!(matches!(
            validate_icc(&icc_fixture(b"spac", b"RGB ")),
            Err(IccError::WrongClass { .. })
        ));
    }

    #[test]
    fn rejects_non_rgb_display_profiles() {
        assert!(matches!(
            validate_icc(&icc_fixture(b"mntr", b"GRAY")),
            Err(IccError::WrongColorSpace { .. })
        ));
    }

    #[test]
    fn enforces_the_32_mib_ceiling_before_reading_the_body() {
        let oversized = vec![0u8; (MAX_ICC_BYTES + 1) as usize];
        assert!(matches!(
            validate_icc(&oversized),
            Err(IccError::TooLarge { .. })
        ));
    }

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // ── manifest ────────────────────────────────────────────────────────────

    #[test]
    fn bundled_catalogue_parses_and_is_internally_consistent() {
        let catalog = catalog().expect("bundled catalogue must parse");
        assert!(catalog.version >= 1);
        assert!(!catalog.entries.is_empty());
        let mut ids = std::collections::HashSet::new();
        for entry in &catalog.entries {
            assert!(ids.insert(entry.id.clone()), "duplicate id {}", entry.id);
            if entry.kind == IccKind::Download {
                // Downloads cross the privilege boundary: strict name only.
                sanitize_profile_file_name(entry.file_name())
                    .unwrap_or_else(|e| panic!("{}: {e}", entry.id));
            } else {
                // Local entries keep the Windows/OEM name, which may contain
                // spaces ("sRGB Color Space Profile.icm"); it never crosses the
                // boundary because the file is already in the system directory.
                let name = entry.file_name();
                assert!(name.ends_with(".icc") || name.ends_with(".icm"));
                assert!(
                    !name.contains('\\') && !name.contains('/'),
                    "{} is not a bare name",
                    entry.id
                );
            }
            assert!(!entry.display_name.is_empty());
            assert!(!entry.source.is_empty());
            if entry.kind == IccKind::Download {
                let sha = entry.sha256.as_deref().expect("download needs sha256");
                assert_eq!(sha.len(), 64, "{} sha256 length", entry.id);
                assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(
                    entry.bytes.is_some_and(|b| b > 0 && b <= MAX_ICC_BYTES),
                    "{} needs a sane pinned size",
                    entry.id
                );
                let url = entry.source_url.as_deref().expect("download needs a URL");
                assert!(url.starts_with("https://"), "{} must use https", entry.id);
                assert!(
                    !entry.license_note.is_empty(),
                    "{} needs a licence note",
                    entry.id
                );
            } else {
                assert!(entry.bytes.is_none(), "local entries must not pin bytes");
            }
        }
    }

    #[test]
    fn catalogue_matches_panel_specific_entries_by_edid() {
        let edo = monitor("EDO", "4503", "");
        let lg = monitor("GSM", "774A", "LG ULTRAGEAR");
        assert_eq!(
            match_quality(find_entry("oem-edo4503-d65p3").unwrap(), Some(&edo)),
            Some(MatchQuality::Exact)
        );
        assert_eq!(
            match_quality(find_entry("oem-edo4503-d65p3").unwrap(), Some(&lg)),
            None,
            "a panel profile must never be offered for a different panel"
        );
        assert_eq!(
            match_quality(find_entry("windows-srgb-iec61966-2-1").unwrap(), Some(&lg)),
            Some(MatchQuality::Generic)
        );
        assert_eq!(
            match_quality(find_entry("windows-srgb-iec61966-2-1").unwrap(), None),
            Some(MatchQuality::Generic)
        );
    }

    #[test]
    fn unknown_panel_only_gets_generic_entries() {
        let dir = std::env::temp_dir();
        let views = entries_for(None, &dir);
        assert!(!views.is_empty());
        assert!(views
            .iter()
            .all(|v| v.match_quality == MatchQuality::Generic));
        assert!(views.iter().all(|v| !v.compatible));
    }

    #[test]
    fn panel_specific_entries_sort_before_generic_ones() {
        let edo = monitor("EDO", "4503", "");
        let views = entries_for(Some(&edo), &std::env::temp_dir());
        let first_compatible = views.iter().position(|v| v.compatible);
        let first_generic = views.iter().position(|v| !v.compatible);
        assert!(first_compatible.is_some());
        assert!(
            first_compatible < first_generic,
            "compatible profiles must come first"
        );
        assert!(views.iter().any(|v| v.file_name == "xiaomi_D65P3_base.icc"));
    }

    // ── file names and staging containment ──────────────────────────────────

    #[test]
    fn accepts_plain_profile_names() {
        for name in ["a.icc", "MiControl-sRGB2014.icc", "sRGB2014.icm"] {
            assert_eq!(sanitize_profile_file_name(name).as_deref(), Ok(name));
        }
    }

    #[test]
    fn rejects_path_traversal_and_odd_extensions() {
        for name in [
            "../evil.icc",
            "..\\evil.icc",
            "a/../b.icc",
            ".hidden.icc",
            "evil.exe",
            "noext",
            "a b.icc",
            "a\0.icc",
        ] {
            assert!(
                sanitize_profile_file_name(name).is_err(),
                "accepted unsafe name {name:?}"
            );
        }
        assert!(sanitize_profile_file_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn refuses_to_install_a_file_that_is_not_in_the_catalogue() {
        // Runs before any filesystem access, so no staging dir is needed.
        let err = install_staged_profile("not-in-catalog.icc").unwrap_err();
        assert!(err.contains("not an installable profile"), "{err}");
    }

    #[test]
    fn refuses_to_install_an_unsafe_name() {
        assert!(install_staged_profile("../evil.icc").is_err());
    }

    /// Network test (opt-in): fetches the real ICC registry profile and checks
    /// the pinned hash, then removes the staged copy.
    #[test]
    #[ignore = "network: downloads the ICC sRGB2014 profile"]
    fn downloads_and_validates_the_real_icc_profile() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let result = runtime
            .block_on(download_entry("icc-srgb2014"))
            .expect("download icc-srgb2014");
        assert_eq!(result.bytes, 3024);
        assert_eq!(
            result.sha256,
            "384b832de3412066743b52a75ee906b6fb9fb8d9e09e936fc2c43223815c6e0a"
        );
        assert!(std::path::Path::new(&result.staging_path).is_file());
        let _ = std::fs::remove_file(&result.staging_path);
    }

    #[test]
    fn recognises_an_already_installed_profile_by_hash() {
        let dir = std::env::temp_dir().join(format!("micontrol-icc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let bytes = icc_fixture(b"mntr", b"RGB ");
        let name = "MiControl-test.icc";
        std::fs::write(dir.join(name), &bytes).expect("fixture");
        let hash = sha256_hex(&bytes);

        let mut entry = find_entry("icc-srgb2014").expect("entry").clone();
        entry.install_name = name.to_string();
        entry.sha256 = Some(hash.clone());
        entry.bytes = Some(bytes.len() as u64);
        assert!(installed_profile_matches(&entry, &dir));

        entry.sha256 = Some("0".repeat(64));
        assert!(
            !installed_profile_matches(&entry, &dir),
            "a different hash must not count as installed"
        );
        assert!(!installed_profile_matches(&entry, &dir.join("missing")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── online research links ───────────────────────────────────────────────

    #[test]
    fn builds_the_two_real_search_urls() {
        assert_eq!(
            profile_search_url("notebookcheck", "EDO4503").unwrap(),
            "https://www.notebookcheck.net/Google-Search.36690.0.html?q=EDO4503"
        );
        assert_eq!(
            profile_search_url("panelook", "LG ULTRAGEAR").unwrap(),
            "https://www.panelook.com/modelsearch.php?keyword=LG%20ULTRAGEAR"
        );
    }

    #[test]
    fn rejects_unusable_search_input() {
        assert!(profile_search_url("panelook", "  ").is_err());
        assert!(profile_search_url("panelook", "a\"b").is_err());
        assert!(profile_search_url("panelook", "a&b").is_err());
        assert!(profile_search_url("panelook", &"x".repeat(81)).is_err());
        assert!(profile_search_url("example", "EDO4503").is_err());
    }
}
