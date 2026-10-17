//! Win32 windowing helpers shared by the send / settings / receive windows.
//!
//! Each window owns a heap-allocated context struct (containing the shared
//! `Arc<AppInner>` plus window-local state) stored in `GWLP_USERDATA`.

pub mod receive_window;
pub mod send_window;
pub mod settings_window;
pub mod text_window;

use crate::util::{pcwstr, to_wide};
use std::sync::OnceLock;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows::Win32::UI::WindowsAndMessaging::*;

// Control-specific style bits, re-typed as `WINDOW_STYLE` so they combine
// cleanly with the `WS_*` constants (which are also `WINDOW_STYLE`). The raw
// `BS_*`/`LBS_*`/`PBS_*`/`ES_*` constants in `Win32::UI::Controls` are plain
// integers and would break the `|` expression's type. Values mirror winuser.h.
pub const BS_PUSHBUTTON: WINDOW_STYLE = WINDOW_STYLE(0x0000_0000);
pub const BS_AUTOCHECKBOX: WINDOW_STYLE = WINDOW_STYLE(0x0000_0003);
pub const LBS_NOTIFY: WINDOW_STYLE = WINDOW_STYLE(0x0000_0001);
pub const LBS_HASSTRINGS: WINDOW_STYLE = WINDOW_STYLE(0x0000_0040);
pub const PBS_SMOOTH: WINDOW_STYLE = WINDOW_STYLE(0x0000_0001);
pub const ES_AUTOHSCROLL: WINDOW_STYLE = WINDOW_STYLE(0x0000_0080);
pub const ES_MULTILINE: WINDOW_STYLE = WINDOW_STYLE(0x0000_0004);
pub const ES_AUTOVSCROLL: WINDOW_STYLE = WINDOW_STYLE(0x0000_0040);
pub const ES_WANTRETURN: WINDOW_STYLE = WINDOW_STYLE(0x0000_1000);

/// Register a window class backed by `wndproc`. Returns the class ATOM (0 on failure).
pub fn register_class(name: &str, wndproc: WNDPROC) -> u16 {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let wname = to_wide(name);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(hinst.0),
            hIcon: load_app_icon(GetSystemMetrics(SM_CXICON)),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(0)),
            hbrBackground: HBRUSH(COLOR_WINDOW.0 as isize + 1),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: pcwstr(&wname),
            hIconSm: load_app_icon(GetSystemMetrics(SM_CXSMICON)),
        };
        RegisterClassExW(&wc)
    }
}

/// Load the embedded application icon (resource id `crate::IDI_APP`) at the given
/// pixel size. `size` of 0 lets the system pick the appropriate resolution from
/// the multi-image `.ico`. Returns `HICON(0)` if the resource is missing.
pub fn load_app_icon(size: i32) -> HICON {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        // `MAKEINTRESOURCEW` is not exported by windows-rs 0.52; a numeric
        // resource id is passed as a synthetic pointer whose low 16 bits are the
        // id (the convention Windows uses to distinguish ids from string names).
        let name = PCWSTR(crate::IDI_APP as usize as *const u16);
        match LoadImageW(hinst, name, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
            Ok(h) => HICON(h.0),
            Err(_) => HICON(0),
        }
    }
}

/// The single UI font, created once and reused for every window / control.
/// Microsoft YaHei is preferred; if it is unavailable (non-Chinese systems) we
/// fall back to Segoe UI, then the system default message font.
static APP_FONT: OnceLock<HFONT> = OnceLock::new();

/// Lazily create (and cache) the UI font. DPI-aware: the point size is scaled by
/// the primary monitor's vertical DPI so the UI looks the same at 100%/125%/150%.
pub fn init_font() -> HFONT {
    *APP_FONT.get_or_init(|| unsafe { create_font() })
}

unsafe fn create_font() -> HFONT {
    let hdc = GetDC(HWND(0));
    let dpi = if hdc.is_invalid() {
        96
    } else {
        let d = GetDeviceCaps(hdc, LOGPIXELSY);
        let _ = ReleaseDC(HWND(0), hdc);
        if d <= 0 {
            96
        } else {
            d
        }
    };
    // 9 point, scaled by DPI. Negative height => character height (not cell).
    let height = -(9 * dpi) / 72;

    let mut lf = LOGFONTW::default();
    lf.lfHeight = height;
    lf.lfWeight = FW_NORMAL.0 as i32;
    lf.lfCharSet = DEFAULT_CHARSET.0 as u8;
    // DEFAULT_QUALITY
    lf.lfQuality = 0;

    for face in ["Microsoft YaHei", "Segoe UI"] {
        let w = to_wide(face);
        for (i, ch) in w.iter().take(31).enumerate() {
            lf.lfFaceName[i] = *ch;
        }
        let f = CreateFontIndirectW(&lf);
        if f.0 != 0 {
            return f;
        }
    }
    HFONT(0)
}

/// Apply the cached UI font to a window (or control) and redraw it.
pub fn set_font(hwnd: HWND) {
    let f = init_font();
    if f.0 != 0 {
        unsafe {
            let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
        }
    }
}

/// Create a child control (button / edit / listbox / static / progress).
pub fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    unsafe {
        let c = to_wide(class);
        let t = to_wide(text);
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap().0);
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            pcwstr(&c),
            pcwstr(&t),
            style | WS_CHILD | WS_VISIBLE,
            x,
            y,
            w,
            h,
            parent,
            HMENU(id as isize),
            hinst,
            None,
        );
        set_font(h);
        h
    }
}

/// Initialise common controls (progress bar, etc.).
pub fn init_common_controls() {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        let _ = InitCommonControlsEx(&icc);
    }
}

/// Borrow the window's context struct.
///
/// # Safety
/// The window must have stored a `Box<T>` in `GWLP_USERDATA`.
pub unsafe fn ctx_from_hwnd<T: 'static>(hwnd: HWND) -> &'static mut T {
    &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T)
}
