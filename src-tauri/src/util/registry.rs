//! Safe wrappers for Windows registry operations.
//!
//! This module replaces the unsafe `std::mem::zeroed()` pattern for HKEY
//! handles with `MaybeUninit`, ensuring that `RegCloseKey` is only called
//! on handles that were successfully initialized.

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    KEY_ALL_ACCESS, KEY_READ, REG_CREATE_KEY_DISPOSITION, REG_OPTION_NON_VOLATILE, REG_VALUE_TYPE,
};

/// A guard that closes the registry key when dropped.
#[allow(dead_code)]
pub struct RegKeyGuard {
    handle: Option<HKEY>,
}

impl RegKeyGuard {
    /// Open a registry key for reading.
    /// Returns `Ok(None)` if the key does not exist (not an error).
    #[allow(dead_code)]
    pub fn open_read(parent: HKEY, subkey: &str) -> Result<Option<Self>, String> {
        let subkey_w: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut handle = std::mem::MaybeUninit::<HKEY>::uninit();
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
    #[allow(dead_code)]
    pub fn create_write(parent: HKEY, subkey: &str) -> Result<Self, String> {
        let subkey_w: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut handle = std::mem::MaybeUninit::<HKEY>::uninit();
        let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
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
    #[allow(dead_code)]
    pub fn read_string(&self, name: &str) -> Result<Option<String>, String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf_len: u32 = 512;
        let mut buf = vec![0u16; (buf_len / 2) as usize];
        let mut value_type = REG_VALUE_TYPE::default();

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
            return Ok(None);
        }

        let len = (buf_len / 2) as usize;
        let s: String = String::from_utf16_lossy(&buf[..len.saturating_sub(1).min(buf.len())]);
        Ok(Some(s.trim_end_matches('\0').to_string()))
    }

    /// Read a u32 value from the registry.
    #[allow(dead_code)]
    pub fn read_u32(&self, name: &str) -> Result<Option<u32>, String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut value: u32 = 0;
        let mut buf_len: u32 = 4;
        let mut value_type = REG_VALUE_TYPE::default();

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
    #[allow(dead_code)]
    pub fn write_u32(&self, name: &str, value: u32) -> Result<(), String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
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

    /// Write a string value to the registry.
    #[allow(dead_code)]
    pub fn write_string(&self, name: &str, value: &str) -> Result<(), String> {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let value_w: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
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

#[allow(dead_code)]
fn ptr_cast_slice<T>(s: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * std::mem::size_of::<T>())
    }
}
