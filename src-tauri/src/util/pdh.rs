//! Single PDH (Performance Data Helper) engine (U5 / D8).
//!
//! One `pdh.dll` load per process (cached in a `OnceLock`), one RAII query
//! type, one typed counter path builder and one status-to-catalog mapping.
//! The pollers (`hw::system_info` CPU + GPU, `hw::taskmgr` engine + network)
//! consume this module instead of each carrying their own libloading
//! boilerplate; the machine governor builds on the same engine later.
//!
//! # Concurrency contract
//!
//! This module holds NO global lock around sampling. The library static is
//! immutable after load and every [`PdhQuery`] owns its own PDH handle, so
//! two pollers sampling at the same time never serialize on each other.
//! A single query is not designed for concurrent use across threads: each
//! poller thread owns its query exclusively.
//!
//! # Error contract
//!
//! PDH status codes never surface raw: [`PdhError`] classifies them into the
//! central error catalog (`errors.catalog`) and converts into [`AppError`].
//! Informational PDH statuses (`PDH_MORE_DATA`, the 0x8000xxxx warning band)
//! are handled inside the helpers and never become errors.

use crate::errors::{AppError, ErrorCode};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// PDH status constants (pdhmsg.h)
// ---------------------------------------------------------------------------

/// PDH format flag: produce `double` values.
const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
/// CStatus: the value is valid.
pub const PDH_CSTATUS_VALID_DATA: u32 = 0x0000_0000;
/// CStatus: the value is valid and newer than the last read.
pub const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;
/// PDH warning: the caller's buffer was too small (size-probe contract).
pub const PDH_MORE_DATA: u32 = 0x8000_07D2;
/// PDH error: the handle is not an open PDH handle.
pub const PDH_INVALID_HANDLE: u32 = 0xC000_0BBC;
// PDH error: the counter name is missing from the path.
pub const PDH_CSTATUS_NO_COUNTERNAME: u32 = 0xC000_0BBF;
// PDH error: the counter path cannot be parsed.
pub const PDH_CSTATUS_BAD_COUNTERNAME: u32 = 0xC000_0BC0;
// PDH error: the caller's buffer pointer was null or misaligned.
pub const PDH_INVALID_BUFFER: u32 = 0xC000_0BC1;
// PDH error: the buffer is too small for the formatted array.
pub const PDH_INSUFFICIENT_BUFFER: u32 = 0xC000_0BC2;
// PDH error: the counter path is malformed.
pub const PDH_INVALID_PATH: u32 = 0xC000_0BC4;
// PDH error: the counter instance is invalid.
pub const PDH_INVALID_INSTANCE: u32 = 0xC000_0BC5;
// PDH error: the counter value cannot be computed (rate counter needs 2
// samples, among other causes).
pub const PDH_INVALID_DATA: u32 = 0xC000_0BC6;
// PDH error: an argument was invalid.
pub const PDH_INVALID_ARGUMENT: u32 = 0xC000_0BBD;
// PDH informational: the counter's machine is not reachable (warning band).
pub const PDH_CSTATUS_NO_MACHINE: u32 = 0x8000_07D0;
// PDH informational: the counter has no live instances right now (warning
// band).
pub const PDH_CSTATUS_NO_INSTANCE: u32 = 0x8000_07D1;
// PDH errors: the counter object or counter does not exist on this machine.
// Live note: the current pdh.dll answers every unresolvable path (bogus
// object, bogus counter, malformed path) with PDH_CSTATUS_NO_OBJECT
// (0xC0000BB8); the constants come from the Windows SDK pdhmsg.h.
pub const PDH_CSTATUS_NO_OBJECT: u32 = 0xC000_0BB8;
pub const PDH_CSTATUS_NO_COUNTER: u32 = 0xC000_0BB9;
// PDH error: the counter data is invalid.
pub const PDH_CSTATUS_INVALID_DATA: u32 = 0xC000_0BBA;
// PDH error: memory allocation failed inside PDH.
pub const PDH_MEMORY_ALLOCATION_FAILURE: u32 = 0xC000_0BBB;

/// Hard cap for one formatted-counter-array allocation. Instance sets above
/// this (pathological) are skipped for the cycle instead of allocated, which
/// is what the old inline guards in the pollers did.
pub const MAX_ARRAY_BYTES: u32 = 64 * 1024 * 1024;

/// Maximum instance count decoded from one array buffer.
const MAX_ARRAY_ITEMS: usize = 4096;

// ---------------------------------------------------------------------------
// Catalog classification
// ---------------------------------------------------------------------------

/// Which catalog class a PDH failure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdhErrorKind {
    /// `pdh.dll` itself is missing or its exports are unavailable.
    LibraryUnavailable,
    /// The query could not be opened or closed.
    QueryFailed,
    /// The counter object/path does not exist on this platform.
    CounterUnavailable,
    /// Collecting or formatting an existing counter failed.
    SampleFailed,
}

impl PdhErrorKind {
    /// The catalog code for this failure class.
    pub fn code(self) -> ErrorCode {
        match self {
            Self::LibraryUnavailable => ErrorCode::NotSupported,
            Self::QueryFailed => ErrorCode::PdhQueryFailed,
            Self::CounterUnavailable => ErrorCode::PdhCounterUnavailable,
            Self::SampleFailed => ErrorCode::PdhSampleFailed,
        }
    }
}

/// A PDH failure with its catalog classification attached.
#[derive(Debug, Clone)]
pub struct PdhError {
    /// Failure class, resolving to a catalog [`ErrorCode`].
    pub kind: PdhErrorKind,
    /// Raw PDH status (`PDH_*`), 0 when no status was involved.
    pub pdh_status: u32,
    /// Where and what failed, in the caller's words (goes to logs).
    pub context: String,
}

impl PdhError {
    fn library_fail(context: impl Into<String>) -> Self {
        Self {
            kind: PdhErrorKind::LibraryUnavailable,
            pdh_status: 0,
            context: context.into(),
        }
    }

    fn open_fail(status: u32, context: impl Into<String>) -> Self {
        Self {
            kind: PdhErrorKind::QueryFailed,
            pdh_status: status,
            context: context.into(),
        }
    }

    fn close_fail(status: u32, context: impl Into<String>) -> Self {
        Self {
            kind: PdhErrorKind::QueryFailed,
            pdh_status: status,
            context: context.into(),
        }
    }

    fn add_fail(status: u32, context: impl Into<String>) -> Self {
        Self {
            kind: classify_counter_status(status),
            pdh_status: status,
            context: context.into(),
        }
    }

    fn sample_fail(status: u32, context: impl Into<String>) -> Self {
        Self {
            kind: classify_sample_status(status),
            pdh_status: status,
            context: context.into(),
        }
    }

    fn bad_path(context: impl Into<String>) -> Self {
        Self {
            kind: PdhErrorKind::QueryFailed,
            pdh_status: 0,
            context: context.into(),
        }
    }
}

impl std::fmt::Display for PdhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            PdhErrorKind::LibraryUnavailable => "library load",
            PdhErrorKind::QueryFailed => "query",
            PdhErrorKind::CounterUnavailable => "counter lookup",
            PdhErrorKind::SampleFailed => "sample",
        };
        write!(
            f,
            "pdh {kind} failed (status 0x{:08X}): {}",
            self.pdh_status, self.context
        )
    }
}

impl std::error::Error for PdhError {}

impl From<PdhError> for AppError {
    fn from(e: PdhError) -> Self {
        AppError::new(e.kind.code(), e.to_string())
    }
}

/// Missing-object style statuses mean the counter is not on this platform.
fn classify_counter_status(status: u32) -> PdhErrorKind {
    match status {
        // Unresolvable paths (bogus object/counter, malformed path, unknown
        // machine or no instances) mean the counter is not available here.
        PDH_CSTATUS_NO_OBJECT
        | PDH_CSTATUS_NO_COUNTER
        | PDH_INVALID_ARGUMENT
        | PDH_CSTATUS_NO_MACHINE
        | PDH_CSTATUS_NO_INSTANCE
        | PDH_CSTATUS_NO_COUNTERNAME
        | PDH_CSTATUS_BAD_COUNTERNAME
        | PDH_INVALID_PATH
        | PDH_INVALID_INSTANCE => PdhErrorKind::CounterUnavailable,
        _ => PdhErrorKind::QueryFailed,
    }
}

/// Unreadable-data style statuses mean the sample failed, not the query.
fn classify_sample_status(status: u32) -> PdhErrorKind {
    match status {
        PDH_INVALID_DATA | PDH_CSTATUS_INVALID_DATA => PdhErrorKind::SampleFailed,
        _ => PdhErrorKind::QueryFailed,
    }
}

/// True for the CStatus values that carry a usable number (0 or 1).
pub fn is_usable_status(status: u32) -> bool {
    status <= PDH_CSTATUS_NEW_DATA
}

// ---------------------------------------------------------------------------
// Typed counter paths
// ---------------------------------------------------------------------------

/// One instance selector inside a [`CounterPath`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CounterInstance {
    /// Every instance of the object: `(*)`.
    Wildcard,
    /// One named instance: `(name)`, e.g. `_Total`.
    Named(String),
}

/// A typed PDH counter path for the local machine.
///
/// Canonical form: `\object(instance)\counter`, `\object(*)\counter` for
/// wildcards and `\object\counter` when the object has no instances. The
/// canonical English names are used with `PdhAddEnglishCounterW`, so paths
/// work independent of the OS display language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterPath {
    object: String,
    instance: Option<CounterInstance>,
    counter: String,
}

impl CounterPath {
    /// Builds an arbitrary local-machine path.
    ///
    /// Rejects the components PDH would reject (empty object/counter,
    /// backslash inside any component, empty named instance) so the failure
    /// happens here with a catalog error instead of inside PDH.
    pub fn new(
        object: &str,
        instance: Option<CounterInstance>,
        counter: &str,
    ) -> Result<Self, PdhError> {
        for (field, value) in [("object", object), ("counter", counter)] {
            if value.is_empty() || value.contains('\\') {
                return Err(PdhError::bad_path(format!(
                    "invalid counter path {field}: {value:?}"
                )));
            }
        }
        if let Some(CounterInstance::Named(name)) = &instance {
            if name.is_empty() || name.contains('\\') {
                return Err(PdhError::bad_path(format!(
                    "invalid counter path instance: {name:?}"
                )));
            }
        }
        Ok(Self {
            object: object.to_string(),
            instance,
            counter: counter.to_string(),
        })
    }

    /// `\Processor Information(_Total)\% Processor Time` (scheduling-based
    /// CPU busy share, matches Task Manager; immune to hybrid-architecture
    /// utility inflation).
    pub fn cpu_processor_time() -> Self {
        Self {
            object: "Processor Information".to_string(),
            instance: Some(CounterInstance::Named("_Total".to_string())),
            counter: "% Processor Time".to_string(),
        }
    }

    /// `\Processor(_Total)\% Processor Time` (legacy provider fallback).
    pub fn cpu_processor_time_legacy() -> Self {
        Self {
            object: "Processor".to_string(),
            instance: Some(CounterInstance::Named("_Total".to_string())),
            counter: "% Processor Time".to_string(),
        }
    }

    /// `\GPU Engine(*)\Utilization Percentage` (one instance per PID x
    /// engine; consumers sum the `engtype_3D` / `engtype_Neural` slices).
    pub fn gpu_engine_utilization() -> Self {
        Self {
            object: "GPU Engine".to_string(),
            instance: Some(CounterInstance::Wildcard),
            counter: "Utilization Percentage".to_string(),
        }
    }

    /// `\Network Interface(*)\Bytes Total/sec` (up+down per interface).
    pub fn network_bytes_total() -> Self {
        Self {
            object: "Network Interface".to_string(),
            instance: Some(CounterInstance::Wildcard),
            counter: "Bytes Total/sec".to_string(),
        }
    }

    /// Canonical path string.
    pub fn as_path(&self) -> String {
        let mut s = String::with_capacity(self.object.len() + self.counter.len() + 4);
        s.push('\\');
        s.push_str(&self.object);
        match &self.instance {
            Some(CounterInstance::Wildcard) => s.push_str("(*)"),
            Some(CounterInstance::Named(name)) => {
                s.push('(');
                s.push_str(name);
                s.push(')');
            }
            None => {}
        }
        s.push('\\');
        s.push_str(&self.counter);
        s
    }

    /// Null-terminated UTF-16 form ready for the PDH wide APIs.
    pub fn to_wide(&self) -> Vec<u16> {
        self.as_path()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }
}

impl std::fmt::Display for CounterPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_path())
    }
}

// ---------------------------------------------------------------------------
// One-time library load
// ---------------------------------------------------------------------------

type FnOpenQuery = unsafe extern "system" fn(*const std::ffi::c_void, u32, *mut isize) -> u32;
type FnAddCounter = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> u32;
type FnCollect = unsafe extern "system" fn(isize) -> u32;
type FnGetFormatted = unsafe extern "system" fn(isize, u32, *mut u32, *mut u8) -> u32;
type FnGetArray = unsafe extern "system" fn(isize, u32, *mut u32, *mut u32, *mut u8) -> u32;
type FnClose = unsafe extern "system" fn(isize) -> u32;

/// Raw `pdh.dll` entry points, loaded once for the process lifetime.
#[derive(Clone, Copy)]
struct PdhLib {
    open_query: FnOpenQuery,
    add_english_counter: FnAddCounter,
    collect: FnCollect,
    get_formatted: FnGetFormatted,
    get_array: FnGetArray,
    close_query: FnClose,
}

static PDH_LIB: OnceLock<Option<PdhLib>> = OnceLock::new();

#[cfg(windows)]
fn load_library() -> Option<PdhLib> {
    // SAFETY: pdh.dll is a system DLL. The Library is leaked so the raw fn
    // pointers stay valid for the process lifetime (this runs once, inside
    // the PDH_LIB OnceLock). `Library::get` resolves the well-known PDH
    // exports; the symbol names are NUL-terminated byte strings.
    // SAFETY: loading a DLL runs its DllMain; pdh.dll is a trusted system
    // component (same call shape the previous per-poller loaders used).
    let lib: &'static libloading::Library = match unsafe { libloading::Library::new("pdh.dll") } {
        Ok(l) => Box::leak(Box::new(l)),
        Err(_) => return None,
    };
    let (open_query, add_english_counter, collect, get_formatted, get_array, close_query) = unsafe {
        let open_query: FnOpenQuery = *lib.get(b"PdhOpenQueryW\0").ok()?;
        let add_english_counter: FnAddCounter = *lib.get(b"PdhAddEnglishCounterW\0").ok()?;
        let collect: FnCollect = *lib.get(b"PdhCollectQueryData\0").ok()?;
        let get_formatted: FnGetFormatted = *lib.get(b"PdhGetFormattedCounterValue\0").ok()?;
        let get_array: FnGetArray = *lib.get(b"PdhGetFormattedCounterArrayW\0").ok()?;
        let close_query: FnClose = *lib.get(b"PdhCloseQuery\0").ok()?;
        (
            open_query,
            add_english_counter,
            collect,
            get_formatted,
            get_array,
            close_query,
        )
    };
    Some(PdhLib {
        open_query,
        add_english_counter,
        collect,
        get_formatted,
        get_array,
        close_query,
    })
}

#[cfg(not(windows))]
fn load_library() -> Option<PdhLib> {
    None
}

/// Process-wide PDH entry point: loads `pdh.dll` once, caches a failed load
/// (callers must not retry forever) and returns the raw entry points.
fn library() -> Option<&'static PdhLib> {
    PDH_LIB.get_or_init(load_library).as_ref()
}

// ---------------------------------------------------------------------------
// RAII query
// ---------------------------------------------------------------------------

/// A handle to one counter inside a [`PdhQuery`]. Valid until the owning
/// query is closed or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdhCounter(isize);

/// One formatted counter value (`PdhGetFormattedCounterValue`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormattedValue {
    /// PDH CStatus of this sample.
    pub status: u32,
    /// Formatted value; meaningful only when [`FormattedValue::is_usable`].
    pub value: f64,
}

impl FormattedValue {
    /// CStatus 0 (valid) or 1 (new data) with a finite number.
    pub fn is_usable(&self) -> bool {
        is_usable_status(self.status) && self.value.is_finite()
    }
}

/// One decoded item from a formatted counter array.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedCounterItem {
    /// Instance name (e.g. `pid_4242_luid_..._engtype_3D`, `Wi-Fi`).
    pub name: String,
    /// PDH CStatus of this item.
    pub status: u32,
    /// Formatted value (PDH_FMT_DOUBLE).
    pub value: f64,
}

/// A live PDH query. Opens on [`PdhQuery::open`], closes on
/// [`PdhQuery::close`] or on `drop` (RAII), so error paths never leak the
/// underlying handle.
pub struct PdhQuery {
    handle: isize,
    closed: bool,
}

impl PdhQuery {
    /// Opens a new PDH query on the local machine.
    pub fn open() -> Result<Self, PdhError> {
        let Some(lib) = library() else {
            return Err(PdhError::library_fail("pdh.dll is not available"));
        };
        let mut handle: isize = 0;
        // SAFETY: handle is the out-parameter; a null data source means the
        // local machine (real-time data source by default).
        let status = unsafe { (lib.open_query)(std::ptr::null(), 0, &mut handle) };
        if status != 0 {
            return Err(PdhError::open_fail(status, "PdhOpenQueryW"));
        }
        Ok(Self {
            handle,
            closed: false,
        })
    }

    /// Adds a counter by its canonical English path
    /// (`PdhAddEnglishCounterW`), independent of the OS display language.
    pub fn add_english_counter(&mut self, path: &CounterPath) -> Result<PdhCounter, PdhError> {
        let Some(lib) = library() else {
            return Err(PdhError::library_fail("pdh.dll is not available"));
        };
        let wide = path.to_wide();
        let mut handle: isize = 0;
        // SAFETY: wide is a null-terminated UTF-16 path alive across the call;
        // handle is the out-parameter.
        let status =
            unsafe { (lib.add_english_counter)(self.handle, wide.as_ptr(), 0, &mut handle) };
        if status != 0 {
            return Err(PdhError::add_fail(
                status,
                format!("PdhAddEnglishCounterW({path})"),
            ));
        }
        Ok(PdhCounter(handle))
    }

    /// Samples every counter in the query once (`PdhCollectQueryData`).
    ///
    /// Rate counters need at least two samples before their values are
    /// readable; the first collect on a fresh counter may legitimately fail
    /// with `PDH_INVALID_DATA` (classified `SampleFailed`).
    pub fn collect(&self) -> Result<(), PdhError> {
        let Some(lib) = library() else {
            return Err(PdhError::library_fail("pdh.dll is not available"));
        };
        // SAFETY: handle is an open query handle.
        let status = unsafe { (lib.collect)(self.handle) };
        if status != 0 {
            return Err(PdhError::sample_fail(status, "PdhCollectQueryData"));
        }
        Ok(())
    }

    /// Reads one counter as a formatted double
    /// (`PdhGetFormattedCounterValue`).
    pub fn formatted_double(&self, counter: PdhCounter) -> Result<FormattedValue, PdhError> {
        let Some(lib) = library() else {
            return Err(PdhError::library_fail("pdh.dll is not available"));
        };
        // PDH_FMT_COUNTERVALUE (x64): CStatus u32 at +0, double value at +8.
        let mut buf = [0u8; 16];
        let mut counter_type: u32 = 0;
        // SAFETY: buf matches the PDH_FMT_COUNTERVALUE layout; counter_type
        // receives the counter's type. All reads go through from_ne_bytes.
        let status = unsafe {
            (lib.get_formatted)(
                counter.0,
                PDH_FMT_DOUBLE,
                &mut counter_type,
                buf.as_mut_ptr(),
            )
        };
        if status != 0 {
            return Err(PdhError::sample_fail(
                status,
                format!("PdhGetFormattedCounterValue(handle {:#x})", counter.0),
            ));
        }
        Ok(FormattedValue {
            status: u32::from_ne_bytes(buf[0..4].try_into().expect("fixed 4-byte slice")),
            value: f64::from_ne_bytes(buf[8..16].try_into().expect("fixed 8-byte slice")),
        })
    }

    /// Reads one multi-instance counter as named items
    /// (`PdhGetFormattedCounterArrayW`, two-call size-probe contract).
    ///
    /// Returns an empty vec when PDH reports no instances this cycle or when
    /// the required buffer exceeds `max_bytes` (skipped, not allocated);
    /// hard API failures map to [`PdhError`].
    pub fn formatted_array(
        &self,
        counter: PdhCounter,
        max_bytes: u32,
    ) -> Result<Vec<NamedCounterItem>, PdhError> {
        let Some(lib) = library() else {
            return Err(PdhError::library_fail("pdh.dll is not available"));
        };
        let mut buf_size: u32 = 0;
        let mut item_count: u32 = 0;
        // SAFETY: null buffer + out-params is the documented size probe.
        let status = unsafe {
            (lib.get_array)(
                counter.0,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                std::ptr::null_mut(),
            )
        };
        if status != PDH_MORE_DATA {
            if buf_size == 0 && item_count == 0 {
                // No instances exist this cycle; not a failure.
                return Ok(Vec::new());
            }
            return Err(PdhError::sample_fail(
                status,
                "PdhGetFormattedCounterArrayW(size probe)",
            ));
        }
        if buf_size == 0 || item_count == 0 {
            return Ok(Vec::new());
        }
        if buf_size > max_bytes {
            // Pathological instance set: skip the cycle instead of allocating.
            return Ok(Vec::new());
        }
        let mut data = vec![0u8; buf_size as usize];
        // SAFETY: data is sized to the buffer size PDH just reported.
        let fill_status = unsafe {
            (lib.get_array)(
                counter.0,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                data.as_mut_ptr(),
            )
        };
        if fill_status != 0 {
            return Err(PdhError::sample_fail(
                fill_status,
                "PdhGetFormattedCounterArrayW(fill)",
            ));
        }
        Ok(decode_counter_array(&data, item_count as usize))
    }

    /// Closes the query explicitly, reporting PDH's close status. The RAII
    /// `drop` path stays as the backstop for error paths.
    pub fn close(mut self) -> Result<(), PdhError> {
        let result = self.close_handle();
        self.closed = true;
        result
    }

    fn close_handle(&mut self) -> Result<(), PdhError> {
        if self.closed || self.handle == 0 {
            return Ok(());
        }
        let Some(lib) = library() else {
            return Err(PdhError::library_fail("pdh.dll is not available"));
        };
        // SAFETY: handle is an open query handle owned by this object.
        let status = unsafe { (lib.close_query)(self.handle) };
        if status != 0 {
            return Err(PdhError::close_fail(status, "PdhCloseQuery"));
        }
        Ok(())
    }
}

impl Drop for PdhQuery {
    fn drop(&mut self) {
        // Best effort: a close failure after Drop is unobservable. PDH frees
        // the query's resources even when PdhCloseQuery reports a status.
        let _ = self.close_handle();
    }
}

// ---------------------------------------------------------------------------
// Formatted array decoding
// ---------------------------------------------------------------------------

/// Decodes a `PdhGetFormattedCounterArrayW` buffer (`PDH_FMT_DOUBLE`) into
/// named items.
///
/// Layout (x64), `PDH_FMT_COUNTERVALUE_ITEM_W`, 24 bytes per item:
/// `+0` szName (pointer to a wide string stored in this buffer), `+8`
/// CStatus, `+12` padding, `+16` f64 value. szName is an absolute pointer on
/// x64 and a relative offset on some builds; both forms are translated.
/// Items with an out-of-range or empty name are skipped (no usable instance).
pub fn decode_counter_array(data: &[u8], item_count: usize) -> Vec<NamedCounterItem> {
    let mut items = Vec::new();
    if item_count == 0 || item_count > MAX_ARRAY_ITEMS {
        return items;
    }
    const ITEM_SIZE: usize = 24;
    if data.len() < item_count * ITEM_SIZE {
        return items;
    }
    let base = data.as_ptr() as usize;
    for i in 0..item_count {
        let off = i * ITEM_SIZE;
        let name_ptr =
            usize::from_le_bytes(data[off..off + 8].try_into().expect("fixed 8-byte slice"));
        let status = u32::from_le_bytes(
            data[off + 8..off + 12]
                .try_into()
                .expect("fixed 4-byte slice"),
        );
        let value = f64::from_le_bytes(
            data[off + 16..off + 24]
                .try_into()
                .expect("fixed 8-byte slice"),
        );
        let name_off = if name_ptr >= base {
            name_ptr - base
        } else {
            name_ptr
        };
        if name_off == 0 || name_off >= data.len() {
            continue;
        }
        let name = read_wide_string(data, name_off);
        if name.is_empty() {
            continue;
        }
        items.push(NamedCounterItem {
            name,
            status,
            value,
        });
    }
    items
}

/// Reads a null-terminated UTF-16 string at `pos` inside a PDH array buffer.
fn read_wide_string(data: &[u8], mut pos: usize) -> String {
    let mut chars: Vec<u16> = Vec::new();
    while pos + 1 < data.len() {
        let unit = u16::from_le_bytes([data[pos], data[pos + 1]]);
        if unit == 0 {
            break;
        }
        chars.push(unit);
        pos += 2;
    }
    String::from_utf16_lossy(&chars)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_path_formats_canonical_and_custom_paths() {
        assert_eq!(
            CounterPath::cpu_processor_time().to_string(),
            r"\Processor Information(_Total)\% Processor Time"
        );
        assert_eq!(
            CounterPath::cpu_processor_time_legacy().to_string(),
            r"\Processor(_Total)\% Processor Time"
        );
        assert_eq!(
            CounterPath::gpu_engine_utilization().to_string(),
            r"\GPU Engine(*)\Utilization Percentage"
        );
        assert_eq!(
            CounterPath::network_bytes_total().to_string(),
            r"\Network Interface(*)\Bytes Total/sec"
        );
        let custom = CounterPath::new(
            "Object X",
            Some(CounterInstance::Named("inst 1".to_string())),
            "Counter Y",
        )
        .expect("valid components");
        assert_eq!(custom.to_string(), r"\Object X(inst 1)\Counter Y");
        let bare = CounterPath::new("Object X", None, "Counter Y").expect("valid components");
        assert_eq!(bare.to_string(), r"\Object X\Counter Y");
    }

    #[test]
    fn counter_path_rejects_invalid_components() {
        assert!(CounterPath::new("", Some(CounterInstance::Wildcard), "c").is_err());
        assert!(CounterPath::new("o", Some(CounterInstance::Wildcard), "").is_err());
        assert!(CounterPath::new(r"o\bj", Some(CounterInstance::Wildcard), "c").is_err());
        assert!(CounterPath::new("o", Some(CounterInstance::Wildcard), r"c\nt").is_err());
        assert!(
            CounterPath::new("o", Some(CounterInstance::Named("a\\b".to_string())), "c").is_err()
        );
        assert!(CounterPath::new("o", Some(CounterInstance::Named(String::new())), "c").is_err());
    }

    #[test]
    fn counter_path_wide_encoding_is_null_terminated() {
        let wide = CounterPath::network_bytes_total().to_wide();
        assert!(wide.len() > 1);
        assert_eq!(*wide.last().expect("non-empty"), 0);
        assert_eq!(
            String::from_utf16_lossy(&wide[..wide.len() - 1]),
            r"\Network Interface(*)\Bytes Total/sec"
        );
    }

    #[test]
    fn pdh_status_maps_to_the_right_catalog_code() {
        // Counter-availability band.
        for status in [
            PDH_CSTATUS_NO_COUNTERNAME,
            PDH_CSTATUS_BAD_COUNTERNAME,
            PDH_INVALID_PATH,
            PDH_INVALID_INSTANCE,
            PDH_INVALID_ARGUMENT,
            PDH_CSTATUS_NO_OBJECT,
            PDH_CSTATUS_NO_COUNTER,
        ] {
            assert_eq!(
                PdhError::add_fail(status, "test").kind.code(),
                ErrorCode::PdhCounterUnavailable,
                "status 0x{status:08X} must classify as counter unavailable"
            );
        }
        // Live-verified: the current pdh.dll answers unresolvable paths with
        // PDH_CSTATUS_NO_OBJECT (0xC0000BB8), covered above.
        for status in [
            PDH_INVALID_HANDLE,
            PDH_MEMORY_ALLOCATION_FAILURE,
            PDH_INSUFFICIENT_BUFFER,
            PDH_INVALID_BUFFER,
        ] {
            assert_eq!(
                PdhError::add_fail(status, "test").kind.code(),
                ErrorCode::PdhQueryFailed,
                "status 0x{status:08X} must classify as query failed"
            );
        }
        // Sampling failures.
        for status in [PDH_INVALID_DATA, PDH_CSTATUS_INVALID_DATA] {
            assert_eq!(
                PdhError::sample_fail(status, "test").kind.code(),
                ErrorCode::PdhSampleFailed,
                "status 0x{status:08X} must classify as sample failed"
            );
        }
        // Open/close always classify as query lifecycle failures.
        assert_eq!(
            PdhError::open_fail(PDH_INVALID_ARGUMENT, "test")
                .kind
                .code(),
            ErrorCode::PdhQueryFailed
        );
        assert_eq!(
            PdhError::close_fail(PDH_INVALID_HANDLE, "test").kind.code(),
            ErrorCode::PdhQueryFailed
        );
        // The library failure carries the platform-unsupported code.
        assert_eq!(
            PdhError::library_fail("test").kind.code(),
            ErrorCode::NotSupported
        );
    }

    #[test]
    fn pdh_error_converts_into_app_error_from_the_catalog() {
        let err = PdhError::add_fail(PDH_CSTATUS_NO_OBJECT, "PdhAddEnglishCounterW(test)");
        let app: AppError = err.clone().into();
        assert_eq!(app.code, ErrorCode::PdhCounterUnavailable);
        assert!(
            app.context.contains("0xC0000BB8"),
            "context must carry the raw status: {}",
            app.context
        );
        assert!(
            err.to_string().contains("PdhAddEnglishCounterW(test)"),
            "display must carry the context: {err}"
        );

        let sample = PdhError::sample_fail(PDH_INVALID_DATA, "first collect");
        let app: AppError = sample.into();
        assert_eq!(app.code, ErrorCode::PdhSampleFailed);
    }

    #[test]
    fn usable_status_band_is_zero_or_one() {
        assert!(is_usable_status(PDH_CSTATUS_VALID_DATA));
        assert!(is_usable_status(PDH_CSTATUS_NEW_DATA));
        assert!(!is_usable_status(2));
        assert!(!is_usable_status(PDH_CSTATUS_INVALID_DATA));
        let usable = FormattedValue {
            status: PDH_CSTATUS_NEW_DATA,
            value: 42.0,
        };
        assert!(usable.is_usable());
        assert!(!FormattedValue {
            status: PDH_CSTATUS_VALID_DATA,
            value: f64::NAN,
        }
        .is_usable());
    }

    /// Builds a synthetic `PDH_FMT_COUNTERVALUE_ITEM_W` buffer: `count` items
    /// of 24 bytes followed by the wide instance strings.
    fn synthetic_buffer(names: &[&str], statuses: &[u32], values: &[f64]) -> Vec<u8> {
        assert_eq!(names.len(), statuses.len());
        assert_eq!(names.len(), values.len());
        let mut data: Vec<u8> = Vec::new();
        let mut name_offsets = Vec::new();
        let strings_off = names.len() * 24;
        let mut cursor = strings_off;
        for name in names {
            name_offsets.push(cursor);
            cursor += name.encode_utf16().count() * 2 + 2;
        }
        for i in 0..names.len() {
            data.extend_from_slice(&(name_offsets[i] as u64).to_le_bytes());
            data.extend_from_slice(&statuses[i].to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes()); // padding
            data.extend_from_slice(&values[i].to_le_bytes());
        }
        for name in names {
            for unit in name.encode_utf16().chain(std::iter::once(0)) {
                data.extend_from_slice(&unit.to_le_bytes());
            }
        }
        data
    }

    #[test]
    fn decode_counter_array_reads_relative_and_absolute_name_pointers() {
        let names = ["pid_1000_engtype_3D", "Wi-Fi"];
        let statuses = [0u32, 1];
        let values = [12.5, 1024.5];
        let data = synthetic_buffer(&names, &statuses, &values);

        let items = decode_counter_array(&data, 2);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, names[0]);
        assert_eq!(items[0].status, 0);
        assert!((items[0].value - 12.5).abs() < 1e-9);
        assert_eq!(items[1].name, names[1]);
        assert_eq!(items[1].status, 1);
        assert!((items[1].value - 1024.5).abs() < 1e-9);

        // Absolute-pointer form: rewrite each szName to base + offset; the
        // decode must translate it back to the same items.
        let mut absolute = data.clone();
        let base = absolute.as_ptr() as usize;
        for i in 0..2 {
            let off = i * 24;
            let relative = u64::from_le_bytes(absolute[off..off + 8].try_into().unwrap());
            let absolute_ptr = (base + relative as usize) as u64;
            absolute[off..off + 8].copy_from_slice(&absolute_ptr.to_le_bytes());
        }
        assert_eq!(decode_counter_array(&absolute, 2), items);
    }

    #[test]
    fn decode_counter_array_rejects_impossible_shapes() {
        assert!(decode_counter_array(&[], 0).is_empty());
        // Buffer shorter than one 24-byte item.
        assert!(decode_counter_array(&[0u8; 8], 1).is_empty());
        // Instance count above the decode cap.
        assert!(decode_counter_array(&vec![0u8; 24 * 5000], 5000).is_empty());
        // In-range item with a null name pointer is skipped.
        let mut one = vec![0u8; 24];
        one[0..8].copy_from_slice(&0u64.to_le_bytes());
        one[16..24].copy_from_slice(&7.0f64.to_le_bytes());
        assert!(decode_counter_array(&one, 1).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn raii_query_closes_on_drop_and_reopens() {
        // Dozens of open/add/collect/drop cycles prove Drop releases the
        // handle: a leak would exhaust PDH's per-process query budget.
        for _ in 0..64 {
            let mut query = PdhQuery::open().expect("open pdh query");
            let counter = query
                .add_english_counter(&CounterPath::cpu_processor_time_legacy())
                .expect("add cpu counter");
            let _ = query.collect(); // first sample may be invalid by design
            drop(query);
            let _ = counter; // handle dies with the query
        }
        // The same path still works after all those drops.
        let query = PdhQuery::open().expect("reopen after drops");
        query.close().expect("close after reopen");
    }

    #[cfg(windows)]
    #[test]
    fn explicit_close_reports_pdhs_status() {
        let mut query = PdhQuery::open().expect("open pdh query");
        let counter = query
            .add_english_counter(&CounterPath::cpu_processor_time_legacy())
            .expect("add cpu counter");
        let _ = query.collect();
        let _ = query.formatted_double(counter);
        query.close().expect("explicit close must succeed");
    }

    #[cfg(windows)]
    #[test]
    fn bogus_counter_path_classifies_as_counter_unavailable() {
        let mut query = PdhQuery::open().expect("open pdh query");
        let path = CounterPath::new("MiControl Nonexistent Object", None, "Nonexistent Counter")
            .expect("valid components");
        let err = query
            .add_english_counter(&path)
            .expect_err("bogus path must fail");
        assert_eq!(
            err.kind.code(),
            ErrorCode::PdhCounterUnavailable,
            "live status 0x{:08X} classified as {:?}",
            err.pdh_status,
            err.kind
        );
        query.close().ok();
    }

    #[cfg(windows)]
    #[test]
    fn cpu_total_counter_reads_between_0_and_100() {
        let mut query = PdhQuery::open().expect("open pdh query");
        let counter = match query.add_english_counter(&CounterPath::cpu_processor_time()) {
            Ok(c) => c,
            Err(_) => query
                .add_english_counter(&CounterPath::cpu_processor_time_legacy())
                .expect("legacy cpu counter"),
        };
        let _ = query.collect(); // baseline
        let mut value = None;
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = query.collect();
            if let Ok(v) = query.formatted_double(counter) {
                if v.is_usable() {
                    value = Some(v.value);
                    break;
                }
            }
        }
        let pct = value.expect("cpu total sample must become valid");
        assert!(
            (0.0..=100.0).contains(&pct),
            "cpu total out of range: {pct}"
        );
        query.close().ok();
    }
}
