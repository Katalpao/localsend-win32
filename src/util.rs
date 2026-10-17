//! Small helpers for Win32 string handling and the legacy Common Dialogs
//! (file open / folder browse) that don't require COM plumbing.

use std::path::PathBuf;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Convert a Rust string into a null-terminated UTF-16 vector.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Build a `PCWSTR` from a wide buffer. The buffer must outlive the pointer.
pub fn pcwstr(wide: &[u16]) -> PCWSTR {
    PCWSTR(wide.as_ptr())
}

/// Read the text of an edit / static / listbox item into a String.
pub fn get_window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len as usize) + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

/// Set the text of any window / control.
pub fn set_window_text(hwnd: HWND, text: &str) {
    unsafe {
        let w = to_wide(text);
        let _ = SetWindowTextW(hwnd, pcwstr(&w));
    }
}

/// Open the classic multi-select file dialog. Returns the chosen paths.
/// `title` and `filter_label` are already localized by the caller.
pub fn open_file_dialog(owner: HWND, multiselect: bool, title: &str, filter_label: &str) -> Option<Vec<PathBuf>> {
    unsafe {
        let mut buf = vec![0u16; 64 * 1024];
        // Build the classic double-null-terminated filter string.
        let filter = to_wide(&format!("{}\0*.*\0\0", filter_label));
        let title_w = to_wide(title);
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: pcwstr(&filter),
            lpstrFile: PWSTR(buf.as_mut_ptr()),
            nMaxFile: buf.len() as u32,
            lpstrTitle: pcwstr(&title_w),
            Flags: OFN_FILEMUSTEXIST,
            ..Default::default()
        };
        if multiselect {
            ofn.Flags |= OFN_ALLOWMULTISELECT | OFN_EXPLORER;
        }
        if GetOpenFileNameW(&mut ofn).as_bool() {
            parse_open_filename(&buf)
        } else {
            None
        }
    }
}

fn parse_open_filename(buf: &[u16]) -> Option<Vec<PathBuf>> {
    let s = String::from_utf16_lossy(buf);
    let s = s.trim_end_matches('\0');
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('\0').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && parts[1..].iter().all(|p| !p.contains('\\') && !p.contains('/')) {
        let dir = parts[0];
        Some(
            parts[1..]
                .iter()
                .map(|f| PathBuf::from(format!("{}\\{}", dir, f)))
                .collect(),
        )
    } else {
        Some(vec![PathBuf::from(parts[0])])
    }
}

/// Open the classic folder browser. Returns the chosen directory.
pub fn browse_folder(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let title_w = to_wide(title);
        let mut display = vec![0u16; 260];
        let mut bi = BROWSEINFOW {
            hwndOwner: owner,
            pszDisplayName: PWSTR(display.as_mut_ptr()),
            lpszTitle: pcwstr(&title_w),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            ..Default::default()
        };
        let pidl = SHBrowseForFolderW(&mut bi);
        if pidl.is_null() {
            return None;
        }
        let mut path = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, &mut path);
        CoTaskMemFree(Some(pidl as *mut core::ffi::c_void));
        if ok.as_bool() {
            Some(PathBuf::from(
                String::from_utf16_lossy(&path).trim_end_matches('\0'),
            ))
        } else {
            None
        }
    }
}

/// Copy UTF-16 text onto the system clipboard.
///
/// MUST be called from a thread that owns a message loop (the UI thread),
/// because the clipboard API requires it. On success the system takes
/// ownership of the allocated memory; on failure we free it ourselves.
pub unsafe fn set_clipboard_unicode(text: &str) -> bool {
    if OpenClipboard(None).is_err() {
        return false;
    }
    let _ = EmptyClipboard();

    let mut w: Vec<u16> = text.encode_utf16().collect();
    w.push(0); // null terminator required by CF_UNICODETEXT
    let size = w.len() * std::mem::size_of::<u16>();

    let h = match GlobalAlloc(GMEM_MOVEABLE, size) {
        Ok(h) => h,
        Err(_) => {
            let _ = CloseClipboard();
            return false;
        }
    };
    let p = GlobalLock(h) as *mut u16;
    if p.is_null() {
        let _ = GlobalFree(h);
        let _ = CloseClipboard();
        return false;
    }
    std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
    let _ = GlobalUnlock(h);

    // `SetClipboardData` takes ownership of the handle on success. CF_UNICODETEXT = 13.
    let ok = SetClipboardData(13u32, HANDLE(h.0 as isize)).is_ok();
    if !ok {
        let _ = GlobalFree(h);
    }
    let _ = CloseClipboard();
    ok
}
