//! "Send text" dialog: type a message and pick a recipient device. The text is
//! delivered as a LocalSend `text/plain` transfer so the official client (and
//! our own receiver) copies it straight to the clipboard.

use crate::msg::*;
use crate::protocol::client;
use crate::protocol::types::DeviceInfo;
use crate::theme::{self, CtlKind};
use crate::ui::{ctx_from_hwnd, set_font, BS_PUSHBUTTON, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN, LBS_HASSTRINGS, LBS_NOTIFY};
use crate::util::{get_window_text, pcwstr, set_window_text, to_wide};
use crate::AppInner;
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const IDC_HINT: i32 = 210;
const IDC_DEV_LABEL: i32 = 211;
const IDC_TEXT_EDIT: i32 = 201;
const IDC_DEVICES: i32 = 202;
const IDC_SEND: i32 = 203;
const IDC_CANCEL: i32 = 204;
const IDC_REFRESH: i32 = 205;

pub struct TextCtx {
    pub app: Arc<AppInner>,
    pub devices: Vec<DeviceInfo>,
}

pub fn create_text_window(app: Arc<AppInner>) {
    unsafe {
        let class_name = to_wide("LocalSendTextCls");
        crate::ui::register_class("LocalSendTextCls", Some(text_wnd_proc));
        let lang = app.lang.lock().unwrap();
        let wname = to_wide(&lang.tr("text.title"));
        drop(lang);
        let devices = app.devices.lock().unwrap().values().cloned().collect::<Vec<_>>();
        let ctx = Box::new(TextCtx {
            app: app.clone(),
            devices,
        });
        let raw = Box::into_raw(ctx);
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap().0);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            pcwstr(&class_name),
            pcwstr(&wname),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            520,
            None,
            None,
            hinst,
            Some(raw as *const core::ffi::c_void),
        );
        set_font(hwnd);
        theme::set_dark_titlebar(hwnd, theme::current_dark());
        *app.text_hwnd.lock().unwrap() = Some(crate::Hwnd(hwnd));
    }
}

extern "system" fn text_wnd_proc(
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
                on_command(hwnd, id);
                LRESULT(0)
            }
            WM_LANG_CHANGED => {
                relocalize(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let ctx = ctx_from_hwnd::<TextCtx>(hwnd);
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
                let ctx = ctx_from_hwnd::<TextCtx>(hwnd);
                *ctx.app.text_hwnd.lock().unwrap() = None;
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                let _ = Box::from_raw(raw as *mut TextCtx);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn on_command(hwnd: HWND, id: i32) {
    unsafe {
        match id {
            IDC_SEND => do_send_text(hwnd),
            IDC_CANCEL => {
                let _ = DestroyWindow(hwnd);
            }
            IDC_REFRESH => {
                let ctx = ctx_from_hwnd::<TextCtx>(hwnd);
                ctx.devices = ctx.app.devices.lock().unwrap().values().cloned().collect();
                populate_devices(hwnd, ctx);
            }
            _ => {}
        }
    }
}

unsafe fn do_send_text(hwnd: HWND) {
    let ctx = ctx_from_hwnd::<TextCtx>(hwnd);
    let text = get_window_text(GetDlgItem(hwnd, IDC_TEXT_EDIT));
    if text.trim().is_empty() {
        return;
    }
    let idx = SendMessageW(
        GetDlgItem(hwnd, IDC_DEVICES),
        LB_GETCURSEL,
        WPARAM(0),
        LPARAM(0),
    )
    .0 as i32;
    if idx < 0 || idx as usize >= ctx.devices.len() {
        return;
    }
    let dev = ctx.devices[idx as usize].clone();
    let app = ctx.app.clone();
    app.rt.spawn(client::send_text(app.clone(), dev, text));
    let _ = DestroyWindow(hwnd);
}

// ---- helpers ------------------------------------------------------------

unsafe fn build_controls(hwnd: HWND) {
    let ctx = ctx_from_hwnd::<TextCtx>(hwnd);
    let lang = ctx.app.lang.lock().unwrap();
    crate::ui::control(
        hwnd,
        "STATIC",
        &lang.tr("text.hint"),
        WS_VISIBLE,
        14,
        12,
        440,
        20,
        IDC_HINT,
    );
    crate::ui::control(
        hwnd,
        "EDIT",
        "",
        WS_VISIBLE
            | WS_BORDER
            | WS_VSCROLL
            | ES_MULTILINE
            | ES_AUTOVSCROLL
            | ES_WANTRETURN,
        14,
        36,
        440,
        250,
        IDC_TEXT_EDIT,
    );
    crate::ui::control(
        hwnd,
        "STATIC",
        &lang.tr("text.device_label"),
        WS_VISIBLE,
        14,
        298,
        440,
        20,
        IDC_DEV_LABEL,
    );
    crate::ui::control(
        hwnd,
        "LISTBOX",
        "",
        WS_VISIBLE | WS_BORDER | LBS_NOTIFY | LBS_HASSTRINGS | WS_VSCROLL,
        14,
        322,
        440,
        70,
        IDC_DEVICES,
    );
    crate::ui::control(hwnd, "BUTTON", &lang.tr("text.send"), WS_VISIBLE | BS_PUSHBUTTON, 14, 400, 100, 30, IDC_SEND);
    crate::ui::control(hwnd, "BUTTON", &lang.tr("text.cancel"), WS_VISIBLE | BS_PUSHBUTTON, 130, 400, 100, 30, IDC_CANCEL);
    crate::ui::control(
        hwnd,
        "BUTTON",
        &lang.tr("text.refresh"),
        WS_VISIBLE | BS_PUSHBUTTON,
        246,
        400,
        100,
        30,
        IDC_REFRESH,
    );
    drop(lang);
    populate_devices(hwnd, ctx);
}

unsafe fn relocalize(hwnd: HWND) {
    let ctx = ctx_from_hwnd::<TextCtx>(hwnd);
    let lang = ctx.app.lang.lock().unwrap();
    set_window_text(hwnd, &lang.tr("text.title"));
    set_window_text(GetDlgItem(hwnd, IDC_HINT), &lang.tr("text.hint"));
    set_window_text(GetDlgItem(hwnd, IDC_DEV_LABEL), &lang.tr("text.device_label"));
    set_window_text(GetDlgItem(hwnd, IDC_SEND), &lang.tr("text.send"));
    set_window_text(GetDlgItem(hwnd, IDC_CANCEL), &lang.tr("text.cancel"));
    set_window_text(GetDlgItem(hwnd, IDC_REFRESH), &lang.tr("text.refresh"));
    drop(lang);
    populate_devices(hwnd, ctx);
}

unsafe fn populate_devices(hwnd: HWND, ctx: &TextCtx) {
    let lang = ctx.app.lang.lock().unwrap();
    let lb = GetDlgItem(hwnd, IDC_DEVICES);
    SendMessageW(lb, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
    for d in &ctx.devices {
        let label = lang.trf(
            "send.device_item",
            &[("alias", &d.alias), ("ip", &d.ip), ("port", &d.port.to_string())],
        );
        let w = to_wide(&label);
        SendMessageW(lb, LB_ADDSTRING, WPARAM(0), LPARAM(pcwstr(&w).0 as isize));
    }
}
