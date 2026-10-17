//! System-following dark mode and themed control colours.
//!
//! We don't skin every common control pixel-perfectly. We:
//!   1. Read the OS app dark/light preference from the Personalize registry key.
//!   2. Toggle the immersive-dark title bar via the DWM attribute.
//!   3. Recolour the client background and static / button / edit / listbox
//!      text & fills through `WM_CTLCOLOR*` and `WM_ERASEBKGND`.
//!   4. React to live theme switches via `WM_SETTINGCHANGE`.
//!
//! The current state is cached in `AppInner::dark_mode` so colour handlers can
//! read it without touching the registry on every paint.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::util::to_wide;

/// Process-wide mirror of the current dark-mode state. Lets the colour handlers
/// read the mode without touching the registry on every paint, and without
/// needing a back-reference to `AppInner`.
static DARK: AtomicBool = AtomicBool::new(false);

/// Current cached dark-mode state.
pub fn current_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// Cached solid brushes for the four themed surfaces. Created once and reused;
/// leaking them for the lifetime of the process is fine.
struct Brushes {
    light_bg: HBRUSH,
    dark_bg: HBRUSH,
    light_edit: HBRUSH,
    dark_edit: HBRUSH,
}

static BRUSHES: OnceLock<Brushes> = OnceLock::new();

fn brushes() -> &'static Brushes {
    BRUSHES.get_or_init(|| unsafe {
        Brushes {
            light_bg: CreateSolidBrush(COLORREF(0x00_F0_F0_F0)),
            dark_bg: CreateSolidBrush(COLORREF(0x00_20_20_20)),
            light_edit: CreateSolidBrush(COLORREF(0x00_FF_FF_FF)),
            dark_edit: CreateSolidBrush(COLORREF(0x00_2B_2B_2B)),
        }
    })
}

/// Themed colours for a given mode. COLORREF is 0x00BBGGRR.
pub struct Colors {
    pub fg: COLORREF,
    pub bg: COLORREF,
    pub edit: COLORREF,
    pub edit_fg: COLORREF,
}

pub fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            fg: COLORREF(0x00_E8_E8_E8),
            bg: COLORREF(0x00_20_20_20),
            edit: COLORREF(0x00_2B_2B_2B),
            edit_fg: COLORREF(0x00_E8_E8_E8),
        }
    } else {
        Colors {
            fg: COLORREF(0x00_00_00_00),
            bg: COLORREF(0x00_F0_F0_F0),
            edit: COLORREF(0x00_FF_FF_FF),
            edit_fg: COLORREF(0x00_00_00_00),
        }
    }
}

/// Which control kind is being coloured (controls the background brush).
pub enum CtlKind {
    Window,
    Edit,
    List,
}

/// Read whether the OS currently uses dark mode for apps.
pub fn system_dark() -> bool {
    unsafe {
        let sub = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let mut hkey = HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        );
        if status.is_err() {
            return false;
        }
        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let val = to_wide("AppsUseLightTheme");
        let s = RegQueryValueExW(
            hkey,
            PCWSTR(val.as_ptr()),
            None,
            None,
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        // AppsUseLightTheme == 0 → dark.
        s.is_ok() && data == 0
    }
}

/// Enable/disable the immersive-dark title bar on a window (Windows 10 1903+).
/// On older builds the call is a no-op (the title bar keeps the classic look).
pub fn set_dark_titlebar(hwnd: HWND, dark: bool) -> bool {
    unsafe {
        let enable: BOOL = if dark { TRUE } else { FALSE };
        let res = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &enable as *const BOOL as *const core::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
        res.is_ok()
    }
}

/// Paint the window's client background with the themed brush.
/// Call from `WM_ERASEBKGND`; returns `true` if the message was handled.
pub fn erase_background(hwnd: HWND, wparam: WPARAM, dark: bool) -> bool {
    unsafe {
        let hdc = HDC(wparam.0 as isize);
        if hdc.is_invalid() {
            return false;
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let br = if dark { brushes().dark_bg } else { brushes().light_bg };
        let _ = FillRect(hdc, &rc, br);
        true
    }
}

/// Handle a `WM_CTLCOLOR*` message. `wparam` is the device context,
/// `lparam` is the control HWND. Returns the brush to use as the background.
pub fn ctl_color(wparam: WPARAM, _lparam: LPARAM, dark: bool, kind: CtlKind) -> LRESULT {
    unsafe {
        let hdc = HDC(wparam.0 as isize);
        let c = colors(dark);
        match kind {
            CtlKind::Edit | CtlKind::List => {
                let _ = SetBkColor(hdc, c.edit);
                let _ = SetTextColor(hdc, c.edit_fg);
                let br = if dark { brushes().dark_edit } else { brushes().light_edit };
                LRESULT(br.0 as isize)
            }
            _ => {
                let _ = SetBkColor(hdc, c.bg);
                let _ = SetTextColor(hdc, c.fg);
                let br = if dark { brushes().dark_bg } else { brushes().light_bg };
                LRESULT(br.0 as isize)
            }
        }
    }
}

/// Re-read the system theme, persist it, restyle the title bar and repaint the
/// window (and its children) so colours update live.
pub fn on_system_setting_change(app: &crate::AppInner, hwnd: HWND) {
    let dark = system_dark();
    DARK.store(dark, Ordering::Relaxed);
    app.dark_mode.store(dark, Ordering::Relaxed);
    set_dark_titlebar(hwnd, dark);
    unsafe {
        let _ = RedrawWindow(
            hwnd,
            None,
            HRGN(0),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
        );
    }
}

/// Initialise the cached dark-mode flag on the shared app state.
pub fn init_dark_mode() -> AtomicBool {
    let dark = system_dark();
    DARK.store(dark, Ordering::Relaxed);
    AtomicBool::new(dark)
}
