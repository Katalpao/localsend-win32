//! System tray icon, right-click menu and balloon notifications.

use crate::AppInner;
use crate::util::to_wide;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const TRAY_ID: u32 = 1;
pub const TRAYM_SETTINGS: u32 = 9001;
pub const TRAYM_QUICKSEND: u32 = 9002;
pub const TRAYM_OPENPATH: u32 = 9003;
pub const TRAYM_EXIT: u32 = 9004;
pub const TRAYM_TEXTSEND: u32 = 9005;

pub const WM_TRAYICON: u32 = crate::msg::WM_TRAYICON;

/// Create and register the tray icon. Returns the icon handle (also cached by caller).
pub fn create_tray_icon(hwnd: HWND, tip: &str) -> HICON {
    unsafe {
        // Use the embedded application icon; fall back to the system default if
        // the resource is somehow unavailable.
        let mut icon = crate::ui::load_app_icon(GetSystemMetrics(SM_CXSMICON));
        if icon.0 == 0 {
            icon = LoadIconW(None, IDI_APPLICATION).unwrap_or(HICON(0));
        }
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAYICON,
            hIcon: icon,
            ..Default::default()
        };
        fill(&mut nid.szTip, tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        // NOTE: We deliberately stay on the *default* notify-icon version (0)
        // instead of opting into NOTIFYICON_VERSION_4. Under v4 the shell
        // delivers a right-click as a bare `WM_CONTEXTMENU` — frequently as a
        // *standalone* window message that bypasses the `uCallbackMessage`
        // callback entirely, so the handler inside `WM_TRAYICON` can never see
        // it (and `NIN_CONTEXTMENU` is not even exported by windows-rs 0.52).
        // On version 0 the right-click is delivered through the callback as
        // `WM_RBUTTONUP` and the left double-click as `WM_LBUTTONDBLCLK` — both
        // are handled deterministically in `send_wnd_proc`.
        icon
    }
}

/// Show the tray context menu at the current cursor position. Menu labels are
/// taken from the active language pack.
pub fn show_tray_menu(hwnd: HWND, app: &AppInner) {
    unsafe {
        let lang = app.lang.lock().unwrap();
        let menu = CreatePopupMenu().unwrap();
        let items = [
            (TRAYM_SETTINGS, lang.tr("tray.menu_settings")),
            (TRAYM_QUICKSEND, lang.tr("tray.menu_quicksend")),
            (TRAYM_TEXTSEND, lang.tr("tray.menu_textsend")),
            (TRAYM_OPENPATH, lang.tr("tray.menu_openpath")),
            (TRAYM_EXIT, lang.tr("tray.menu_exit")),
        ];
        for (id, label) in items {
            let w = to_wide(&label);
            let _ = AppendMenuW(menu, MF_STRING, id as usize, windows::core::PCWSTR(w.as_ptr()));
        }
        drop(lang);
        let mut pt = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_LEFTALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

/// Pop a balloon tip from the tray icon.
pub fn balloon(hwnd: HWND, icon: HICON, title: &str, text: &str) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            uFlags: NIF_INFO | NIF_ICON,
            hIcon: icon,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        // Union field: uTimeout (milliseconds).
        nid.Anonymous.uTimeout = 4000;
        fill(&mut nid.szInfo, text);
        fill(&mut nid.szInfoTitle, title);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Remove the tray icon (call on exit).
pub fn remove_tray_icon(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn fill(buf: &mut [u16], s: &str) {
    let w = to_wide(s);
    for (i, ch) in w.iter().enumerate() {
        if i < buf.len() {
            buf[i] = *ch;
        } else {
            break;
        }
    }
}
