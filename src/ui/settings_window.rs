//! Settings panel: alias, ports, download folder, PIN, encryption toggle, and UI language.

use crate::i18n;
use crate::msg::WM_LANG_CHANGED;
use crate::theme::{self, CtlKind};
use crate::ui::{ctx_from_hwnd, set_font, BS_AUTOCHECKBOX, BS_PUSHBUTTON, ES_AUTOHSCROLL};
use crate::util::{browse_folder, get_window_text, pcwstr, set_window_text, to_wide};
use crate::AppInner;
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const IDC_ALIAS: i32 = 200;
const IDC_PORT: i32 = 201;
const IDC_MPORT: i32 = 202;
const IDC_DIR: i32 = 203;
const IDC_BROWSE: i32 = 204;
const IDC_PIN: i32 = 205;
const IDC_ENC: i32 = 206;
const IDC_SAVE: i32 = 207;
const IDC_CANCEL: i32 = 208;
const IDC_NOTE: i32 = 209;
const IDC_LANG: i32 = 210;
// Static labels (given real ids so they can be re-localised on language switch).
const IDC_LBL_ALIAS: i32 = 220;
const IDC_LBL_PORT: i32 = 221;
const IDC_LBL_MPORT: i32 = 222;
const IDC_LBL_DIR: i32 = 223;
const IDC_LBL_PIN: i32 = 224;
const IDC_LBL_ENC: i32 = 225;
const IDC_LBL_LANG: i32 = 226;

pub struct SettingsCtx {
    pub app: Arc<AppInner>,
}

pub fn create_settings_window(app: Arc<AppInner>) -> HWND {
    let class_name = to_wide("LocalSendSettingsCls");
    crate::ui::register_class("LocalSendSettingsCls", Some(settings_wnd_proc));
    let lang = app.lang.lock().unwrap();
    let wname = to_wide(&lang.tr("settings.title"));
    drop(lang);
    let ctx = Box::new(SettingsCtx { app: app.clone() });
    let raw = Box::into_raw(ctx);
    unsafe {
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap().0);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            pcwstr(&class_name),
            pcwstr(&wname),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            440,
            400,
            None,
            None,
            hinst,
            Some(raw as *const core::ffi::c_void),
        );
        set_font(hwnd);
        theme::set_dark_titlebar(hwnd, theme::current_dark());
        hwnd
    }
}

extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = lparam.0 as *const CREATESTRUCTW;
                let raw = (*cs).lpCreateParams;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
                build_controls(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as i32;
                if (wparam.0 >> 16) as u16 != 0 {
                    return LRESULT(0);
                }
                match id {
                    IDC_BROWSE => {
                        let lang = {
                            let ctx = ctx_from_hwnd::<SettingsCtx>(hwnd);
                            ctx.app.lang.lock().unwrap().tr("settings.browse_title")
                        };
                        if let Some(dir) = browse_folder(hwnd, &lang) {
                            let dir_s = dir.to_string_lossy().to_string();
                            set_window_text(GetDlgItem(hwnd, IDC_DIR), &dir_s);
                        }
                    }
                    IDC_SAVE => save(hwnd),
                    IDC_CANCEL => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_LANG_CHANGED => {
                relocalize(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let ctx = ctx_from_hwnd::<SettingsCtx>(hwnd);
                theme::on_system_setting_change(&ctx.app, hwnd);
                LRESULT(0)
            }
            WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                theme::ctl_color(wparam, lparam, theme::current_dark(), CtlKind::Window)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                theme::ctl_color(wparam, lparam, theme::current_dark(), CtlKind::Edit)
            }
            WM_ERASEBKGND => {
                if theme::erase_background(hwnd, wparam, theme::current_dark()) {
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_DESTROY => {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                let _ = Box::from_raw(raw as *mut SettingsCtx);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn build_controls(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SettingsCtx>(hwnd);
        let lang = ctx.app.lang.lock().unwrap();
        let cfg = ctx.app.config.lock().unwrap();

        label(hwnd, &lang.tr("settings.alias"), IDC_LBL_ALIAS, 14, 14);
        edit(hwnd, &cfg.alias, IDC_ALIAS, 14, 34, 400, 22);

        label(hwnd, &lang.tr("settings.port"), IDC_LBL_PORT, 14, 66);
        edit(hwnd, &cfg.port.to_string(), IDC_PORT, 14, 86, 180, 22);

        label(hwnd, &lang.tr("settings.mport"), IDC_LBL_MPORT, 210, 66);
        edit(hwnd, &cfg.multicast_port.to_string(), IDC_MPORT, 210, 86, 180, 22);

        label(hwnd, &lang.tr("settings.dir"), IDC_LBL_DIR, 14, 118);
        edit(hwnd, &cfg.download_dir.to_string_lossy().to_string(), IDC_DIR, 14, 138, 330, 22);
        crate::ui::control(hwnd, "BUTTON", &lang.tr("settings.browse"), WS_VISIBLE | BS_PUSHBUTTON, 350, 136, 64, 26, IDC_BROWSE);

        label(hwnd, &lang.tr("settings.pin"), IDC_LBL_PIN, 14, 170);
        edit(hwnd, &cfg.pin, IDC_PIN, 14, 190, 400, 22);

        let enc = crate::ui::control(
            hwnd,
            "BUTTON",
            &lang.tr("settings.enc"),
            WS_VISIBLE | BS_AUTOCHECKBOX,
            14,
            224,
            400,
            24,
            IDC_ENC,
        );
        SendMessageW(
            enc,
            BM_SETCHECK,
            WPARAM(if cfg.require_encryption { 1 } else { 0 }),
            LPARAM(0),
        );

        label(hwnd, &lang.tr("settings.language"), IDC_LBL_LANG, 14, 262);
        let cb = crate::ui::control(
            hwnd,
            "COMBOBOX",
            "",
            WS_VISIBLE | WS_CHILD | WS_BORDER | WS_VSCROLL | CBS_DROPDOWNLIST,
            14,
            282,
            240,
            200,
            IDC_LANG,
        );
        for (code, name) in i18n::Lang::available() {
            let w = to_wide(&name);
            SendMessageW(cb, CB_ADDSTRING, WPARAM(0), LPARAM(pcwstr(&w).0 as isize));
        }
        let current = cfg.language.clone();
        drop(cfg);
        let langs = i18n::Lang::available();
        let sel = langs.iter().position(|(c, _)| *c == current).unwrap_or(0);
        SendMessageW(cb, CB_SETCURSEL, WPARAM(sel), LPARAM(0));

        crate::ui::control(hwnd, "BUTTON", &lang.tr("settings.save"), WS_VISIBLE | BS_PUSHBUTTON, 120, 320, 90, 30, IDC_SAVE);
        crate::ui::control(hwnd, "BUTTON", &lang.tr("settings.cancel"), WS_VISIBLE | BS_PUSHBUTTON, 230, 320, 90, 30, IDC_CANCEL);
        crate::ui::control(hwnd, "STATIC", &lang.tr("settings.note"), WS_VISIBLE, 14, 360, 400, 20, IDC_NOTE);
    }
}

fn label(hwnd: HWND, text: &str, id: i32, x: i32, y: i32) {
    crate::ui::control(hwnd, "STATIC", text, WS_VISIBLE, x, y, 360, 20, id);
}

fn edit(hwnd: HWND, text: &str, id: i32, x: i32, y: i32, w: i32, h: i32) -> HWND {
    crate::ui::control(hwnd, "EDIT", text, WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL, x, y, w, h, id)
}

/// Update all visible, language-dependent text (used on `WM_LANG_CHANGED`).
unsafe fn relocalize(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SettingsCtx>(hwnd);
        let lang = ctx.app.lang.lock().unwrap();
        set_window_text(hwnd, &lang.tr("settings.title"));
        set_window_text(GetDlgItem(hwnd, IDC_LBL_ALIAS), &lang.tr("settings.alias"));
        set_window_text(GetDlgItem(hwnd, IDC_LBL_PORT), &lang.tr("settings.port"));
        set_window_text(GetDlgItem(hwnd, IDC_LBL_MPORT), &lang.tr("settings.mport"));
        set_window_text(GetDlgItem(hwnd, IDC_LBL_DIR), &lang.tr("settings.dir"));
        set_window_text(GetDlgItem(hwnd, IDC_LBL_PIN), &lang.tr("settings.pin"));
        set_window_text(GetDlgItem(hwnd, IDC_ENC), &lang.tr("settings.enc"));
        set_window_text(GetDlgItem(hwnd, IDC_LBL_LANG), &lang.tr("settings.language"));
        set_window_text(GetDlgItem(hwnd, IDC_BROWSE), &lang.tr("settings.browse"));
        set_window_text(GetDlgItem(hwnd, IDC_SAVE), &lang.tr("settings.save"));
        set_window_text(GetDlgItem(hwnd, IDC_CANCEL), &lang.tr("settings.cancel"));
        // Rebuild the language list (new packs may have appeared) and reselect.
        let cb = GetDlgItem(hwnd, IDC_LANG);
        SendMessageW(cb, CB_RESETCONTENT, WPARAM(0), LPARAM(0));
        for (code, name) in i18n::Lang::available() {
            let w = to_wide(&name);
            SendMessageW(cb, CB_ADDSTRING, WPARAM(0), LPARAM(pcwstr(&w).0 as isize));
        }
        let cur = ctx.app.config.lock().unwrap().language.clone();
        let langs = i18n::Lang::available();
        let sel = langs.iter().position(|(c, _)| *c == cur).unwrap_or(0);
        SendMessageW(cb, CB_SETCURSEL, WPARAM(sel), LPARAM(0));
        set_window_text(GetDlgItem(hwnd, IDC_NOTE), &lang.tr("settings.note"));
        drop(lang);
    }
}

unsafe fn save(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SettingsCtx>(hwnd);
        let alias = get_window_text(GetDlgItem(hwnd, IDC_ALIAS));
        let port_s = get_window_text(GetDlgItem(hwnd, IDC_PORT));
        let mport_s = get_window_text(GetDlgItem(hwnd, IDC_MPORT));
        let dir_s = get_window_text(GetDlgItem(hwnd, IDC_DIR));
        let pin = get_window_text(GetDlgItem(hwnd, IDC_PIN));
        let enc = SendMessageW(GetDlgItem(hwnd, IDC_ENC), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1;

        let idx = SendMessageW(GetDlgItem(hwnd, IDC_LANG), CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as i32;
        let new_lang = i18n::Lang::available()
            .get(idx as usize)
            .map(|(c, _)| c.clone());

        let mut cfg = ctx.app.config.lock().unwrap();
        cfg.alias = alias.trim().to_string();
        if let Ok(p) = port_s.trim().parse::<u16>() {
            cfg.port = p;
        }
        if let Ok(p) = mport_s.trim().parse::<u16>() {
            cfg.multicast_port = p;
        }
        let _ = std::fs::create_dir_all(&dir_s);
        cfg.download_dir = std::path::PathBuf::from(dir_s.trim());
        cfg.pin = pin.trim().to_string();
        cfg.require_encryption = enc;

        let lang_changed = match &new_lang {
            Some(c) if *c != cfg.language => {
                cfg.language = c.clone();
                true
            }
            _ => false,
        };
        let _ = cfg.save();
        drop(cfg);

        if lang_changed {
            let loaded = i18n::Lang::load(&ctx.app.config.lock().unwrap().language);
            *ctx.app.lang.lock().unwrap() = loaded;
            let send = ctx.app.send_hwnd.lock().unwrap().0;
            let _ = PostMessageW(send, WM_LANG_CHANGED, WPARAM(0), LPARAM(0));
        }

        let lang = ctx.app.lang.lock().unwrap();
        set_window_text(GetDlgItem(hwnd, IDC_NOTE), &lang.tr("settings.saved"));
        drop(lang);
    }
}
