//! The main "send" window. Doubles as the persistent main window (closing it
//! hides to tray rather than exiting) and hosts the tray icon / context menu.

use crate::msg::*;
use crate::protocol::client;
use crate::protocol::types::DeviceInfo;
use crate::theme::{self, CtlKind};
use crate::tray;
use crate::ui::receive_window;
use crate::ui::settings_window;
use crate::ui::text_window;
use crate::ui::{
    ctx_from_hwnd, set_font, BS_PUSHBUTTON, LBS_HASSTRINGS, LBS_NOTIFY, PBS_SMOOTH,
};
use crate::util::{open_file_dialog, pcwstr, set_window_text, to_wide};
use crate::AppInner;
use std::path::PathBuf;
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::Controls::{PBM_SETPOS, PBM_SETRANGE32};
use windows::Win32::UI::WindowsAndMessaging::*;

const IDC_DEVICES: i32 = 100;
const IDC_FILES: i32 = 101;
const IDC_ADD: i32 = 102;
const IDC_SEND: i32 = 103;
const IDC_REFRESH: i32 = 104;
const IDC_SETTINGS: i32 = 105;
const IDC_STATUS: i32 = 106;
const IDC_PROGRESS: i32 = 107;
const IDC_DEV_LABEL: i32 = 108;
const IDC_FILE_LABEL: i32 = 109;
const IDC_TEXT: i32 = 110;

pub struct SendCtx {
    pub app: Arc<AppInner>,
    pub files: Vec<PathBuf>,
    pub devices: Vec<DeviceInfo>,
}

pub fn create_send_window(app: Arc<AppInner>) -> HWND {
    let class_name = to_wide("LocalSendSendCls");
    crate::ui::register_class("LocalSendSendCls", Some(send_wnd_proc));
    let lang = app.lang.lock().unwrap();
    let wname = to_wide(&lang.tr("send.title"));
    drop(lang);
    let ctx = Box::new(SendCtx {
        app: app.clone(),
        files: Vec::new(),
        devices: Vec::new(),
    });
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
            680,
            520,
            None,
            None,
            hinst,
            Some(raw as *const core::ffi::c_void),
        );
        set_font(hwnd);
        theme::set_dark_titlebar(hwnd, theme::current_dark());
        *app.send_hwnd.lock().unwrap() = crate::Hwnd(hwnd);
        hwnd
    }
}

extern "system" fn send_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
                let code = ((wparam.0 >> 16) & 0xffff) as u16;
                on_command(hwnd, id, code);
                LRESULT(0)
            }
            WM_TRAYICON => {
                match lparam.0 as u32 {
                    x if x == WM_RBUTTONUP as u32
                        || x == WM_RBUTTONDOWN as u32
                        || x == WM_CONTEXTMENU as u32 =>
                    {
                        let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                        tray::show_tray_menu(hwnd, &ctx.app);
                    }
                    x if x == WM_LBUTTONDBLCLK as u32
                        || x == WM_LBUTTONUP as u32
                        || x == NIN_SELECT as u32 =>
                    {
                        let _ = ShowWindow(hwnd, SW_SHOW);
                        let _ = SetForegroundWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TRANSFER_REQUEST => {
                let inc = Box::from_raw(lparam.0 as *mut IncomingTransfer);
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                let lang = ctx.app.lang.lock().unwrap();
                tray::balloon(
                    hwnd,
                    ctx.app.tray_icon.lock().unwrap().0,
                    &lang.tr("toast.request_title"),
                    &lang.trf(
                        "toast.request_text",
                        &[
                            ("alias", &inc.sender.alias),
                            ("count", &inc.files.len().to_string()),
                        ],
                    ),
                );
                drop(lang);
                receive_window::create_receive_window(ctx.app.clone(), inc);
                LRESULT(0)
            }
            WM_DEVICE_DISCOVERED => {
                let info = Box::from_raw(lparam.0 as *mut DeviceInfo);
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                if let Some(slot) = ctx.devices.iter_mut().find(|d| d.id() == info.id()) {
                    *slot = (*info).clone();
                } else {
                    ctx.devices.push((*info).clone());
                }
                populate_devices(hwnd, ctx);
                LRESULT(0)
            }
            WM_TRANSFER_PROGRESS => {
                let upd = Box::from_raw(lparam.0 as *mut ProgressUpdate);
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                if upd.outgoing {
                    set_progress(hwnd, upd.bytes, upd.total);
                    let lang = ctx.app.lang.lock().unwrap();
                    set_status(
                        hwnd,
                        &lang.trf(
                            "send.status_progress",
                            &[
                                ("file", &upd.file_name),
                                ("bytes", &upd.bytes.to_string()),
                                ("total", &upd.total.to_string()),
                            ],
                        ),
                    );
                    drop(lang);
                } else {
                    let sid = upd.session_id.clone();
                    route_incoming(&ctx.app, &sid, WM_UPDATE_PROGRESS, upd);
                }
                LRESULT(0)
            }
            WM_TRANSFER_COMPLETE => {
                let done = Box::from_raw(lparam.0 as *mut TransferComplete);
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                let is_text = done.is_text;
                let lang = ctx.app.lang.lock().unwrap();
                let (title, text) = if done.outgoing {
                    (
                        lang.tr("send.status_done"),
                        if done.is_text {
                            lang.trf("toast.sent_text", &[("alias", &done.peer_alias)])
                        } else {
                            lang.trf(
                                "toast.sent_files",
                                &[
                                    ("alias", &done.peer_alias),
                                    ("count", &done.file_count.to_string()),
                                ],
                            )
                        },
                    )
                } else if is_text {
                    (
                        lang.tr("toast.text_received_title"),
                        lang.trf("toast.received_text", &[("alias", &done.peer_alias)]),
                    )
                } else {
                    (
                        lang.tr("toast.receive_done_title"),
                        lang.trf(
                            "toast.received_files",
                            &[
                                ("alias", &done.peer_alias),
                                ("count", &done.file_count.to_string()),
                                ("dir", &done.dest_dir),
                            ],
                        ),
                    )
                };
                tray::balloon(hwnd, ctx.app.tray_icon.lock().unwrap().0, &title, &text);
                if !done.outgoing {
                    let sid = done.session_id.clone();
                    route_incoming(&ctx.app, &sid, WM_TRANSFER_DONE, done);
                } else {
                    set_status(hwnd, &lang.tr("send.status_done"));
                    set_progress(hwnd, 100, 100);
                }
                drop(lang);
                LRESULT(0)
            }
            WM_TRANSFER_FAILED => {
                let fail = Box::from_raw(lparam.0 as *mut TransferFailed);
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                let lang = ctx.app.lang.lock().unwrap();
                tray::balloon(
                    hwnd,
                    ctx.app.tray_icon.lock().unwrap().0,
                    &lang.tr("toast.failed_title"),
                    &fail.reason,
                );
                if fail.outgoing {
                    set_status(hwnd, &format!("{}: {}", lang.tr("toast.failed_title"), fail.reason));
                }
                drop(lang);
                LRESULT(0)
            }
            WM_COPY_CLIPBOARD_TEXT => {
                let s = Box::from_raw(lparam.0 as *mut String);
                let _ = crate::util::set_clipboard_unicode(&s);
                LRESULT(0)
            }
            WM_LANG_CHANGED => {
                relocalize(hwnd);
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                for w in ctx.app.incoming_windows.lock().unwrap().values().copied() {
                    let _ = PostMessageW(w.0, WM_LANG_CHANGED, WPARAM(0), LPARAM(0));
                }
                if let Some(t) = ctx.app.text_hwnd.lock().unwrap().clone() {
                    let _ = PostMessageW(t.0, WM_LANG_CHANGED, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
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
            WM_CLOSE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_DESTROY => {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                let _ = Box::from_raw(raw as *mut SendCtx);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn on_command(hwnd: HWND, id: i32, code: u16) {
    if code != 0 {
        return; // only act on BN_CLICKED / menu commands
    }
    unsafe {
        match id {
            IDC_ADD => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                let (title, filter) = {
                    let lang = ctx.app.lang.lock().unwrap();
                    (lang.tr("dlg.open_title"), lang.tr("dlg.all_files"))
                };
                if let Some(files) = open_file_dialog(hwnd, true, &title, &filter) {
                    for f in &files {
                        if !ctx.files.contains(f) {
                            ctx.files.push(f.clone());
                        }
                    }
                    populate_files(hwnd, ctx);
                }
            }
            IDC_SEND => do_send(hwnd),
            IDC_TEXT => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                text_window::create_text_window(ctx.app.clone());
            }
            IDC_REFRESH => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                let devs = ctx.app.devices.lock().unwrap().values().cloned().collect::<Vec<_>>();
                ctx.devices = devs;
                populate_devices(hwnd, ctx);
            }
            IDC_SETTINGS => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                settings_window::create_settings_window(ctx.app.clone());
            }
            _ => handle_tray_menu(hwnd, id),
        }
    }
}

fn handle_tray_menu(hwnd: HWND, id: i32) {
    unsafe {
        match id {
            x if x == tray::TRAYM_SETTINGS as i32 => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                settings_window::create_settings_window(ctx.app.clone());
            }
            x if x == tray::TRAYM_QUICKSEND as i32 => quick_send(hwnd),
            x if x == tray::TRAYM_OPENPATH as i32 => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                let dir = ctx.app.config.lock().unwrap().download_dir.clone();
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
            x if x == tray::TRAYM_TEXTSEND as i32 => {
                let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
                text_window::create_text_window(ctx.app.clone());
            }
            x if x == tray::TRAYM_EXIT as i32 => {
                tray::remove_tray_icon(hwnd);
                PostQuitMessage(0);
            }
            _ => {}
        }
    }
}

fn quick_send(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
        let (title, filter) = {
            let lang = ctx.app.lang.lock().unwrap();
            (lang.tr("dlg.open_title"), lang.tr("dlg.all_files"))
        };
        if let Some(files) = open_file_dialog(hwnd, true, &title, &filter) {
            let app = ctx.app.clone();
            let last = app.config.lock().unwrap().last_target_id.clone();
            if let Some(tid) = last {
                if let Some(dev) = app.devices.lock().unwrap().get(&tid).cloned() {
                    app.rt.spawn(client::send_files(app.clone(), dev, files));
                    return;
                }
            }
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

unsafe fn do_send(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
        let idx = SendMessageW(GetDlgItem(hwnd, IDC_DEVICES), LB_GETCURSEL, WPARAM(0), LPARAM(0)).0
            as i32;
        if idx < 0 || idx as usize >= ctx.devices.len() {
            let lang = ctx.app.lang.lock().unwrap();
            set_status(hwnd, &lang.tr("send.err_no_device"));
            return;
        }
        if ctx.files.is_empty() {
            let lang = ctx.app.lang.lock().unwrap();
            set_status(hwnd, &lang.tr("send.err_no_files"));
            return;
        }
        let dev = ctx.devices[idx as usize].clone();
        let files = ctx.files.clone();
        let app = ctx.app.clone();
        let lang = app.lang.lock().unwrap();
        set_status(
            hwnd,
            &lang.trf(
                "send.status_sending",
                &[
                    ("alias", &dev.alias),
                    ("count", &files.len().to_string()),
                ],
            ),
        );
        drop(lang);
        set_progress(hwnd, 0, 100);
        app.rt.spawn(client::send_files(app.clone(), dev, files));
    }
}

// ---- helpers ------------------------------------------------------------

unsafe fn build_controls(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
        let lang = ctx.app.lang.lock().unwrap();

        set_window_text(GetDlgItem(hwnd, IDC_DEV_LABEL), &lang.tr("send.devices"));
        set_window_text(GetDlgItem(hwnd, IDC_FILE_LABEL), &lang.tr("send.files"));

        crate::ui::control(
            hwnd,
            "LISTBOX",
            "",
            WS_VISIBLE | WS_BORDER | LBS_NOTIFY | LBS_HASSTRINGS | WS_VSCROLL,
            14,
            36,
            320,
            300,
            IDC_DEVICES,
        );
        crate::ui::control(
            hwnd,
            "LISTBOX",
            "",
            WS_VISIBLE | WS_BORDER | LBS_HASSTRINGS | WS_VSCROLL,
            350,
            36,
            310,
            300,
            IDC_FILES,
        );

        crate::ui::control(hwnd, "BUTTON", &lang.tr("send.add"), WS_VISIBLE | BS_PUSHBUTTON, 350, 344, 90, 30, IDC_ADD);
        crate::ui::control(hwnd, "BUTTON", &lang.tr("send.send"), WS_VISIBLE | BS_PUSHBUTTON, 448, 344, 90, 30, IDC_SEND);
        crate::ui::control(hwnd, "BUTTON", &lang.tr("send.refresh"), WS_VISIBLE | BS_PUSHBUTTON, 546, 344, 90, 30, IDC_REFRESH);
        crate::ui::control(hwnd, "BUTTON", &lang.tr("send.settings"), WS_VISIBLE | BS_PUSHBUTTON, 14, 344, 80, 30, IDC_SETTINGS);
        crate::ui::control(hwnd, "BUTTON", &lang.tr("send.send_text"), WS_VISIBLE | BS_PUSHBUTTON, 100, 344, 100, 30, IDC_TEXT);

        crate::ui::control(hwnd, "STATIC", &lang.tr("send.status_label"), WS_VISIBLE, 14, 384, 640, 20, IDC_STATUS);
        let pb = crate::ui::control(
            hwnd,
            "msctls_progress32",
            "",
            WS_VISIBLE | PBS_SMOOTH,
            14,
            410,
            640,
            22,
            IDC_PROGRESS,
        );
        SendMessageW(pb, PBM_SETRANGE32, WPARAM(0), LPARAM(100));
        SendMessageW(pb, PBM_SETPOS, WPARAM(0), LPARAM(0));

        drop(lang);
        ctx.devices = ctx.app.devices.lock().unwrap().values().cloned().collect();
        populate_devices(hwnd, ctx);
        populate_files(hwnd, ctx);
    }
}

/// Update all visible, language-dependent text without recreating the window.
unsafe fn relocalize(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<SendCtx>(hwnd);
        let lang = ctx.app.lang.lock().unwrap();
        set_window_text(hwnd, &lang.tr("send.title"));
        set_window_text(GetDlgItem(hwnd, IDC_DEV_LABEL), &lang.tr("send.devices"));
        set_window_text(GetDlgItem(hwnd, IDC_FILE_LABEL), &lang.tr("send.files"));
        set_window_text(GetDlgItem(hwnd, IDC_STATUS), &lang.tr("send.status_label"));
        set_window_text(GetDlgItem(hwnd, IDC_ADD), &lang.tr("send.add"));
        set_window_text(GetDlgItem(hwnd, IDC_SEND), &lang.tr("send.send"));
        set_window_text(GetDlgItem(hwnd, IDC_REFRESH), &lang.tr("send.refresh"));
        set_window_text(GetDlgItem(hwnd, IDC_SETTINGS), &lang.tr("send.settings"));
        set_window_text(GetDlgItem(hwnd, IDC_TEXT), &lang.tr("send.send_text"));
        drop(lang);
        populate_devices(hwnd, ctx);
        populate_files(hwnd, ctx);
    }
}

unsafe fn populate_devices(hwnd: HWND, ctx: &SendCtx) {
    unsafe {
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
}

unsafe fn populate_files(hwnd: HWND, ctx: &SendCtx) {
    unsafe {
        let lb = GetDlgItem(hwnd, IDC_FILES);
        SendMessageW(lb, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
        for f in &ctx.files {
            let label = f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let w = to_wide(&label);
            SendMessageW(lb, LB_ADDSTRING, WPARAM(0), LPARAM(pcwstr(&w).0 as isize));
        }
        let lang = ctx.app.lang.lock().unwrap();
        set_status(hwnd, &lang.trf("send.status_files_selected", &[("count", &ctx.files.len().to_string())]));
    }
}

unsafe fn set_status(hwnd: HWND, text: &str) {
    let st = GetDlgItem(hwnd, IDC_STATUS);
    set_window_text(st, text);
}

unsafe fn set_progress(hwnd: HWND, bytes: u64, total: u64) {
    unsafe {
        let pb = GetDlgItem(hwnd, IDC_PROGRESS);
        let pct = if total == 0 { 0 } else { ((bytes * 100) / total) as i32 };
        let pct = pct.clamp(0, 100);
        SendMessageW(pb, PBM_SETPOS, WPARAM(pct as usize), LPARAM(0));
    }
}

/// Forward a payload to the window handling a given incoming session.
fn route_incoming<T>(app: &AppInner, session_id: &str, msg: u32, payload: Box<T>) {
    let w = app.incoming_windows.lock().unwrap().get(session_id).copied();
    match w {
        Some(hwnd) => {
            let _ = unsafe {
                PostMessageW(hwnd.0, msg, WPARAM(0), LPARAM(Box::into_raw(payload) as isize))
            };
        }
        None => drop(payload),
    }
}
