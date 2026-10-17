//! LocalSend for Windows — native Win32 + windows-rs reimplementation.
//!
//! Architecture:
//!   * A single-threaded Win32 UI thread runs the message loop and owns every
//!     window and the tray icon.
//!   * A separate Tokio runtime thread runs the LAN discovery, the receiving
//!     HTTP server, and any outgoing transfers.
//!   * The two sides never share HWNDs directly: the async side boxes payloads
//!     and posts them via `PostMessageW`; the UI side takes ownership and frees
//!     them. See `msg.rs`.

#![windows_subsystem = "windows"]

mod config;
mod i18n;
mod msg;
mod protocol;
mod theme;
mod tray;
mod ui;
mod util;

/// Resource id of the embedded application icon (`res/app.rc`). Used to load the
/// icon at runtime for windows and the tray notification.
pub const IDI_APP: u16 = 101;

use config::Config;
use protocol::discovery;
use protocol::server;
use protocol::types::DeviceInfo;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// `HWND` is not `Send` in windows-rs, but we only ever touch it from the UI
/// thread (or hand it to `PostMessageW`, which is safe cross-thread). Wrap it so
/// the shared `AppInner` can be moved onto the async runtime.
#[derive(Clone, Copy)]
pub struct Hwnd(pub HWND);
unsafe impl Send for Hwnd {}
unsafe impl Sync for Hwnd {}

/// Same reasoning as `Hwnd`.
#[derive(Clone, Copy)]
pub struct Hicon(pub HICON);
unsafe impl Send for Hicon {}
unsafe impl Sync for Hicon {}

/// Shared application state. One `Arc<AppInner>` is handed to both the UI and
/// the async runtime.
pub struct AppInner {
    pub config: Mutex<Config>,
    pub fingerprint: String,
    pub devices: Mutex<HashMap<String, DeviceInfo>>,
    /// HWND of the main (send) window — the hub for all UI messages.
    pub send_hwnd: Mutex<Hwnd>,
    /// Maps an incoming session id to its progress window.
    pub incoming_windows: Mutex<HashMap<String, Hwnd>>,
    /// Optional handle of the open "send text" dialog, so a language switch can
    /// reach it. `None` when closed.
    pub text_hwnd: Mutex<Option<Hwnd>>,
    /// Tray icon handle (used for balloon notifications).
    pub tray_icon: Mutex<Hicon>,
    /// Loaded UI language pack.
    pub lang: Mutex<i18n::Lang>,
    /// Cached system dark-mode flag (refreshed on `WM_SETTINGCHANGE`).
    pub dark_mode: AtomicBool,
    pub http: reqwest::Client,
    pub rt: tokio::runtime::Handle,
}

impl AppInner {
    /// Post a custom message (with a boxed payload pointer) to the UI thread.
    pub fn post(&self, message: u32, ptr: isize) {
        let hwnd = self.send_hwnd.lock().unwrap().0;
        if hwnd.0 != 0 {
            unsafe {
                let _ = PostMessageW(hwnd, message, WPARAM(0), LPARAM(ptr));
            }
        }
    }

    /// Record a discovered device and notify the UI to refresh its list.
    pub fn upsert_device(&self, info: DeviceInfo) {
        self.devices.lock().unwrap().insert(info.id(), info.clone());
        self.post(msg::WM_DEVICE_DISCOVERED, Box::into_raw(Box::new(info)) as isize);
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }
}

fn main() {
    init_logging();

    let config = Config::load();
    log::info!("LocalSend-Win32 starting; alias={}", config.alias);

    let lang = i18n::Lang::load(&config.language);
    log::info!("Loaded language pack '{}' ({})", lang.code, lang.name);

    let fingerprint = uuid::Uuid::new_v4().to_string();
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // peers use self-signed certs
        .build()
        .expect("reqwest client");

    // Build the Tokio runtime and grab a handle so the UI can spawn tasks.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let app = Arc::new(AppInner {
        config: Mutex::new(config),
        fingerprint,
        devices: Mutex::new(HashMap::new()),
        send_hwnd: Mutex::new(Hwnd(HWND(0))),
        incoming_windows: Mutex::new(HashMap::new()),
        text_hwnd: Mutex::new(None),
        tray_icon: Mutex::new(Hicon(HICON(0))),
        lang: Mutex::new(lang),
        dark_mode: theme::init_dark_mode(),
        http,
        rt: rt.handle().clone(),
    });

    // Run networking on its own thread.
    let net_app = app.clone();
    std::thread::spawn(move || {
        rt.block_on(async move {
            let d = tokio::spawn(discovery::run_discovery(net_app.clone()));
            let s = tokio::spawn(server::run_server(net_app.clone()));
            let _ = tokio::join!(d, s);
        });
    });

    // Build the UI.
    ui::init_common_controls();
    let hwnd = ui::send_window::create_send_window(app.clone());
    *app.send_hwnd.lock().unwrap() = Hwnd(hwnd);

    let alias = app.config.lock().unwrap().alias.clone();
    let icon = tray::create_tray_icon(hwnd, &alias);
    *app.tray_icon.lock().unwrap() = Hicon(icon);

    log::info!("UI ready; entering message loop");
    message_loop();
}

fn message_loop() {
    unsafe {
        let mut msg = MSG::default();
        loop {
            // GetMessageW returns BOOL: 0 (false) means WM_QUIT (or error).
            let got = GetMessageW(&mut msg, HWND(0), 0, 0);
            if !got.as_bool() {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // Tray exit path reaches here; force-stop the async runtime thread.
    std::process::exit(0);
}

fn init_logging() {
    let path = Config::path().with_file_name("localsend.log");
    if let Ok(file) = std::fs::File::create(&path) {
        let _ = simplelog::WriteLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default(), file);
    }
}
