//! Incoming-transfer confirmation dialog + live progress window.
//!
//! Created when a `prepare-upload` arrives. The user accepts/rejects; while
//! files stream in, the progress bar updates via `WM_UPDATE_PROGRESS`.

use crate::msg::*;
use crate::protocol::types::FileMeta;
use crate::theme::{self, CtlKind};
use crate::ui::{ctx_from_hwnd, set_font, BS_PUSHBUTTON, PBS_SMOOTH};
use crate::util::{pcwstr, set_window_text, to_wide};
use crate::AppInner;
use std::sync::Arc;
use tokio::sync::watch;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{PBM_SETPOS, PBM_SETRANGE32};
use windows::Win32::UI::WindowsAndMessaging::*;

const IDC_ACCEPT: i32 = 300;
const IDC_REJECT: i32 = 301;
const IDC_PROGRESS: i32 = 302;
const IDC_STATUS: i32 = 303;
const IDC_FILES: i32 = 304;
const IDC_TIP: i32 = 305;

pub struct ReceiveCtx {
    pub app: Arc<AppInner>,
    pub sender_alias: String,
    pub session_id: String,
    pub decision_tx: watch::Sender<TransferDecision>,
    pub pb: HWND,
    pub status: HWND,
    pub accept_btn: HWND,
    pub reject_btn: HWND,
    #[allow(dead_code)]
    pub files: Vec<FileMeta>,
}

pub fn create_receive_window(app: Arc<AppInner>, incoming: Box<IncomingTransfer>) -> HWND {
    let class_name = to_wide("LocalSendReceiveCls");
    crate::ui::register_class("LocalSendReceiveCls", Some(receive_wnd_proc));
    let lang = app.lang.lock().unwrap();
    let title = to_wide(&lang.trf("receive.title", &[("alias", &incoming.sender.alias)]));
    drop(lang);
    let session_id = incoming.session_id.clone();
    let decision_tx = incoming.decision_tx.clone();
    let files = incoming.files.clone();

    let ctx = Box::new(ReceiveCtx {
        app: app.clone(),
        sender_alias: incoming.sender.alias.clone(),
        session_id: session_id.clone(),
        decision_tx,
        pb: HWND(0),
        status: HWND(0),
        accept_btn: HWND(0),
        reject_btn: HWND(0),
        files,
    });
    let raw = Box::into_raw(ctx);
    unsafe {
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap().0);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            pcwstr(&class_name),
            pcwstr(&title),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            460,
            360,
            None,
            None,
            hinst,
            Some(raw as *const core::ffi::c_void),
        );
        set_font(hwnd);
        theme::set_dark_titlebar(hwnd, theme::current_dark());
        app.incoming_windows.lock().unwrap().insert(session_id, crate::Hwnd(hwnd));
        hwnd
    }
}

extern "system" fn receive_wnd_proc(
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
                let ctx = ctx_from_hwnd::<ReceiveCtx>(hwnd);
                match id {
                    IDC_ACCEPT => {
                        let _ = ctx.decision_tx.send(TransferDecision::Accepted);
                        let _ = ShowWindow(ctx.accept_btn, SW_HIDE);
                        let _ = ShowWindow(ctx.reject_btn, SW_HIDE);
                        let lang = ctx.app.lang.lock().unwrap();
                        set_window_text(ctx.status, &lang.tr("receive.receiving"));
                        drop(lang);
                    }
                    IDC_REJECT => {
                        let _ = ctx.decision_tx.send(TransferDecision::Rejected);
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_UPDATE_PROGRESS => {
                let upd = Box::from_raw(lparam.0 as *mut ProgressUpdate);
                let ctx = ctx_from_hwnd::<ReceiveCtx>(hwnd);
                let pct = if upd.total == 0 {
                    0
                } else {
                    ((upd.bytes * 100) / upd.total) as i32
                }
                .clamp(0, 100);
                SendMessageW(ctx.pb, PBM_SETPOS, WPARAM(pct as usize), LPARAM(0));
                let lang = ctx.app.lang.lock().unwrap();
                set_window_text(
                    ctx.status,
                    &lang.trf(
                        "receive.progress",
                        &[
                            ("file", &upd.file_name),
                            ("bytes", &upd.bytes.to_string()),
                            ("total", &upd.total.to_string()),
                        ],
                    ),
                );
                drop(lang);
                LRESULT(0)
            }
            WM_TRANSFER_DONE => {
                let _ = Box::from_raw(lparam.0 as *mut TransferComplete);
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_LANG_CHANGED => {
                relocalize(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let ctx = ctx_from_hwnd::<ReceiveCtx>(hwnd);
                theme::on_system_setting_change(&ctx.app, hwnd);
                LRESULT(0)
            }
            WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                theme::ctl_color(wparam, lparam, theme::current_dark(), CtlKind::Window)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                theme::ctl_color(wparam, lparam, theme::current_dark(), CtlKind::List)
            }
            WM_ERASEBKGND => {
                if theme::erase_background(hwnd, wparam, theme::current_dark()) {
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_DESTROY => {
                let ctx = ctx_from_hwnd::<ReceiveCtx>(hwnd);
                ctx.app.incoming_windows.lock().unwrap().remove(&ctx.session_id);
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                let _ = Box::from_raw(raw as *mut ReceiveCtx);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn build_controls(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<ReceiveCtx>(hwnd);
        let lang = ctx.app.lang.lock().unwrap();
        let tip = lang.trf(
            "receive.tip",
            &[
                ("alias", &ctx.sender_alias),
                ("count", &ctx.files.len().to_string()),
            ],
        );
        let _t = crate::ui::control(hwnd, "STATIC", &tip, WS_VISIBLE, 14, 12, 420, 20, IDC_TIP);

        let lb = crate::ui::control(
            hwnd,
            "LISTBOX",
            "",
            WS_VISIBLE | WS_BORDER | WS_VSCROLL,
            14,
            38,
            420,
            200,
            IDC_FILES,
        );
        for f in &ctx.files {
            let is_text = f
                .file_type
                .as_deref()
                .map(|t| t == "text" || t.starts_with("text/"))
                .unwrap_or(false);
            let line = if is_text {
                lang.trf("receive.text_item", &[("bytes", &f.size.to_string())])
            } else {
                lang.trf(
                    "receive.file_item",
                    &[("name", &f.file_name), ("bytes", &f.size.to_string())],
                )
            };
            let w = to_wide(&line);
            SendMessageW(lb, LB_ADDSTRING, WPARAM(0), LPARAM(pcwstr(&w).0 as isize));
        }

        ctx.accept_btn = crate::ui::control(
            hwnd,
            "BUTTON",
            &lang.tr("receive.accept"),
            WS_VISIBLE | BS_PUSHBUTTON,
            120,
            250,
            100,
            32,
            IDC_ACCEPT,
        );
        ctx.reject_btn = crate::ui::control(
            hwnd,
            "BUTTON",
            &lang.tr("receive.reject"),
            WS_VISIBLE | BS_PUSHBUTTON,
            240,
            250,
            100,
            32,
            IDC_REJECT,
        );
        ctx.status = crate::ui::control(hwnd, "STATIC", &lang.tr("receive.waiting"), WS_VISIBLE, 14, 292, 420, 20, IDC_STATUS);
        ctx.pb = crate::ui::control(
            hwnd,
            "msctls_progress32",
            "",
            WS_VISIBLE | PBS_SMOOTH,
            14,
            318,
            420,
            22,
            IDC_PROGRESS,
        );
        SendMessageW(ctx.pb, PBM_SETRANGE32, WPARAM(0), LPARAM(100));
        SendMessageW(ctx.pb, PBM_SETPOS, WPARAM(0), LPARAM(0));
    }
}

/// Re-localise visible text when the language changes at runtime.
unsafe fn relocalize(hwnd: HWND) {
    unsafe {
        let ctx = ctx_from_hwnd::<ReceiveCtx>(hwnd);
        let lang = ctx.app.lang.lock().unwrap();
        set_window_text(
            hwnd,
            &lang.trf("receive.title", &[("alias", &ctx.sender_alias)]),
        );
        set_window_text(
            GetDlgItem(hwnd, IDC_TIP),
            &lang.trf(
                "receive.tip",
                &[
                    ("alias", &ctx.sender_alias),
                    ("count", &ctx.files.len().to_string()),
                ],
            ),
        );
        set_window_text(GetDlgItem(hwnd, IDC_ACCEPT), &lang.tr("receive.accept"));
        set_window_text(GetDlgItem(hwnd, IDC_REJECT), &lang.tr("receive.reject"));
        set_window_text(GetDlgItem(hwnd, IDC_STATUS), &lang.tr("receive.waiting"));
        // Rebuild the file list.
        let lb = GetDlgItem(hwnd, IDC_FILES);
        SendMessageW(lb, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
        for f in &ctx.files {
            let is_text = f
                .file_type
                .as_deref()
                .map(|t| t == "text" || t.starts_with("text/"))
                .unwrap_or(false);
            let line = if is_text {
                lang.trf("receive.text_item", &[("bytes", &f.size.to_string())])
            } else {
                lang.trf(
                    "receive.file_item",
                    &[("name", &f.file_name), ("bytes", &f.size.to_string())],
                )
            };
            let w = to_wide(&line);
            SendMessageW(lb, LB_ADDSTRING, WPARAM(0), LPARAM(pcwstr(&w).0 as isize));
        }
        drop(lang);
    }
}
