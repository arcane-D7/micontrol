//! Safe wrappers for Windows registry operations.
//!
//! Replaces unsafe `std::mem::zeroed()` patterns with `MaybeUninit`,
//! ensuring `RegCloseKey` is only called on successfully initialised handles.
//!
//! `RegKeyGuard` and its methods are a public utility API. Not all methods are
//! currently called within this crate, but they are available for future
//! registry operations without requiring API changes.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, KEY_ALL_ACCESS, KEY_READ, REG_CREATE_KEY_DISPOSITION, REG_OPTION_NON_VOLATILE,
    REG_VALUE_TYPE,
};

/// A guard that closes the registry key when dropped.
pub struct RegKeyGuard {
    handle: Option<HKEY>,
}

impl RegKeyGuard {
    /// Open a registry key for reading.
    /// Returns `Ok(None)` if the key does not exist (not an error).
    pub fn open_read(parent: HKEY, subkey: &str) -> Result<Option<Self>, String> {
        let subkey_w: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut handle = std::mem::MaybeUninit::<HKEY>::uninit();
        // SAFETY: subkey_w is a null-terminated wide string owned by this function.
        // RegOpenKeyExW writes to the MaybeUninit<HKEY> only on success; we check
        // result.is_err() before calling assume_init.
        let result = unsafe {
            RegOpenKeyExW(
                parent,
                PCWSTR(subkey_w.as_ptr()),
                0,
                KEY_READ,
                handle.as_mut_ptr(),
            )
        };
        if result.is_err() {
            // Key doesn't exist or can't be opened — return None
            return Ok(None);
        }
        // SAFETY: RegOpenKeyExW succeeded, so the handle is now initialized.
        let handle = unsafe { handle.assume_init() };
        Ok(Some(Self {
            handle: Some(handle),
        }))
    }

    /// Open or create a registry key for writing.
    pub fn create_write(parent: HKEY, subkey: &str) -> Result<Self, String> {
        let subkey_w: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut handle = std::mem::MaybeUninit::<HKEY>::uninit();
        let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
        // SAFETY: subkey_w is a null-terminated wide string owned by this function.
        // RegCreateKeyExW writes to the MaybeUninit<HKEY> only on success; we check
        // result.is_err() before calling assume_init.
        let result = unsafe {
            RegCreateKeyExW(
                parent,
                PCWSTR(subkey_w.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                None,
                handle.as_mut_ptr(),
                Some(&mut disposition),
            )
        };
        if result.is_err() {
            return Err(format!("RegCreateKeyExW failed: {result:?}"));
        }
        // SAFETY: RegCreateKeyExW succeeded, so the handle is now initialized.
        let handle = unsafe { handle.assume_init() };
        Ok(Self {
            handle: Some(handle),
        })
    }

    /// Get the raw HKEY for passing to registry APIs.
    pub fn as_raw(&self) -> HKEY {
        self.handle.unwrap_or_default()
    }

    /// Read a string value from the registry.
    pub fn read_string(&self, name: &str) -> Result<Option<String>, String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf_len: u32 = 512;
        let mut buf = vec![0u16; (buf_len / 2) as usize];
        let mut value_type = REG_VALUE_TYPE::default();

        // SAFETY: name_w is a null-terminated wide string; buf is a Vec<u16> with
        // sufficient capacity. RegQueryValueExW writes to buf and buf_len; we only
        // read buf after checking result.is_ok().
        let result = unsafe {
            RegQueryValueExW(
                self.as_raw(),
                PCWSTR(name_w.as_ptr()),
                None,
                Some(&mut value_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut buf_len),
            )
        };
        if result.is_err() {
            // S23-003: Check for ERROR_MORE_DATA (code 234) — the value is larger
            // than our initial 512-byte buffer. Retry with the required size.
            const ERROR_MORE_DATA: u32 = 234;

            if result.0 != ERROR_MORE_DATA {
                return Ok(None);
            }

            // buf_len now contains the required size in bytes.
            // Cap at 64KB to prevent unbounded allocation from malicious values.
            const MAX_REGISTRY_VALUE_SIZE: u32 = 64 * 1024;
            if buf_len > MAX_REGISTRY_VALUE_SIZE {
                log::warn!(
                    "Registry value '{}' exceeds 64KB limit ({} bytes), skipping",
                    name,
                    buf_len
                );
                return Ok(None);
            }

            // Reallocate with the required size and retry.
            buf = vec![0u16; (buf_len as usize).div_ceil(2)];
            let result2 = unsafe {
                RegQueryValueExW(
                    self.as_raw(),
                    PCWSTR(name_w.as_ptr()),
                    None,
                    Some(&mut value_type),
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut buf_len),
                )
            };
            if result2.is_err() {
                return Ok(None);
            }
        }

        let len = (buf_len / 2) as usize;
        let s: String = String::from_utf16_lossy(&buf[..len.saturating_sub(1).min(buf.len())]);
        Ok(Some(s.trim_end_matches('\0').to_string()))
    }

    /// Read a u32 value from the registry.
    pub fn read_u32(&self, name: &str) -> Result<Option<u32>, String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut value: u32 = 0;
        let mut buf_len: u32 = 4;
        let mut value_type = REG_VALUE_TYPE::default();

        // SAFETY: name_w is a null-terminated wide string; value is a stack-local
        // u32 with valid alignment. RegQueryValueExW writes to value only on success.
        let result = unsafe {
            RegQueryValueExW(
                self.as_raw(),
                PCWSTR(name_w.as_ptr()),
                None,
                Some(&mut value_type),
                Some(&mut value as *mut u32 as *mut u8),
                Some(&mut buf_len),
            )
        };
        if result.is_err() {
            return Ok(None);
        }
        Ok(Some(value))
    }

    /// Write a u32 value to the registry.
    pub fn write_u32(&self, name: &str, value: u32) -> Result<(), String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: name_w is a null-terminated wide string; value.to_ne_bytes()
        // is a stack-local byte array with valid alignment for REG_DWORD.
        let result = unsafe {
            RegSetValueExW(
                self.as_raw(),
                PCWSTR(name_w.as_ptr()),
                0,
                windows::Win32::System::Registry::REG_DWORD,
                Some(&value.to_ne_bytes()),
            )
        };
        if result.is_err() {
            return Err(format!("RegSetValueExW failed: {result:?}"));
        }
        Ok(())
    }

    /// Enumerate the immediate subkey names of this key.
    ///
    /// Used for capability probes that must not hardcode device-class GUIDs
    /// (e.g. finding the `ComputeAccelerator` class and its device instances).
    pub fn enum_subkeys(&self) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        let mut index = 0u32;
        loop {
            // Device-class keys hold short names ("0000", "Properties"); 256
            // UTF-16 code units is well above the 255-char registry limit.
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            // SAFETY: buf is a stack array of 256 u16 and len is its length in
            // characters. RegEnumKeyExW writes at most len characters and
            // updates len with the count written (excluding the NUL).
            let result = unsafe {
                RegEnumKeyExW(
                    self.as_raw(),
                    index,
                    PWSTR(buf.as_mut_ptr()),
                    &mut len,
                    None,
                    PWSTR::null(),
                    None,
                    None,
                )
            };
            if result.is_err() {
                // ERROR_NO_MORE_ITEMS (259) terminates the enumeration; any
                // other error means we could not read the key.
                const ERROR_NO_MORE_ITEMS: u32 = 259;
                if result.0 == ERROR_NO_MORE_ITEMS {
                    return Ok(names);
                }
                return Err(format!("RegEnumKeyExW failed: {result:?}"));
            }
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
            index += 1;
        }
    }

    /// Write a string value to the registry.
    pub fn write_string(&self, name: &str, value: &str) -> Result<(), String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let value_w: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: name_w and value_w are null-terminated wide strings owned by
        // this function. ptr_cast_slice reinterprets value_w as a byte slice for
        // RegSetValueExW, which copies the data without retaining the pointer.
        let result = unsafe {
            RegSetValueExW(
                self.as_raw(),
                PCWSTR(name_w.as_ptr()),
                0,
                windows::Win32::System::Registry::REG_SZ,
                Some(ptr_cast_slice(&value_w)),
            )
        };
        if result.is_err() {
            return Err(format!("RegSetValueExW failed: {result:?}"));
        }
        Ok(())
    }
}

impl Drop for RegKeyGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: handle was successfully initialized by RegOpenKeyExW or RegCreateKeyExW.
            unsafe {
                let _ = RegCloseKey(handle);
            }
        }
    }
}

/// Reinterpret a slice of `T` as a byte slice for registry APIs that expect `&[u8]`.
fn ptr_cast_slice<T>(s: &[T]) -> &[u8] {
    // SAFETY: s is a valid slice with known length; casting to *const u8 and
    // using size_of_val gives a correct byte view of the same memory. The
    // returned slice borrows s and cannot outlive it.
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    #[test]
    fn enumerates_subkeys_of_a_known_key() {
        let key = RegKeyGuard::open_read(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        )
        .expect("opening a readable key must not error")
        .expect("CurrentVersion must exist");

        let names = key.enum_subkeys().expect("enumeration must succeed");
        assert!(!names.is_empty(), "CurrentVersion always has subkeys");
    }

    #[test]
    fn a_missing_key_opens_as_none() {
        let key = RegKeyGuard::open_read(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\MiControl\ThisKeyDoesNotExist\Nope",
        )
        .expect("a missing key is not an error");
        assert!(key.is_none());
    }
}
