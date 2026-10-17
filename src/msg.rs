//! Custom window messages and the payloads that travel from the async
//! networking runtime to the (single-threaded) Win32 UI thread.
//!
//! The UI thread owns all windows, so the tokio runtime never touches any HWND
//! directly. Instead it boxes a payload and posts its raw pointer via
//! `PostMessageW`; the window procedure takes ownership with `Box::from_raw`
//! and frees it. This keeps cross-thread sharing safe.

use crate::protocol::types::{DeviceInfo, FileMeta};
use tokio::sync::watch;

/// Base for our private messages.
pub const WM_USER_BASE: u32 = 0x4000 + 0x100; // avoid clashing with common controls

/// A new or updated device was discovered on the LAN.
/// LPARAM -> *mut DeviceInfo
pub const WM_DEVICE_DISCOVERED: u32 = WM_USER_BASE + 1;

/// An incoming transfer request arrived; show the confirm dialog.
/// LPARAM -> *mut IncomingTransfer
pub const WM_TRANSFER_REQUEST: u32 = WM_USER_BASE + 2;

/// Progress update for an active (incoming or outgoing) transfer.
/// LPARAM -> *mut ProgressUpdate
pub const WM_TRANSFER_PROGRESS: u32 = WM_USER_BASE + 3;

/// A transfer finished (success). Show a balloon / toast.
/// LPARAM -> *mut TransferComplete
pub const WM_TRANSFER_COMPLETE: u32 = WM_USER_BASE + 4;

/// A transfer failed.
/// LPARAM -> *mut TransferFailed
pub const WM_TRANSFER_FAILED: u32 = WM_USER_BASE + 5;

/// Tray icon callback.
pub const WM_TRAYICON: u32 = WM_USER_BASE + 6;

/// Routed to a specific incoming-transfer window to refresh its progress bar.
/// LPARAM -> *mut ProgressUpdate
pub const WM_UPDATE_PROGRESS: u32 = WM_USER_BASE + 9;

/// Routed to a specific incoming-transfer window to close it (done/failed).
/// LPARAM -> *mut TransferComplete  (or TransferFailed repurposed)
pub const WM_TRANSFER_DONE: u32 = WM_USER_BASE + 10;

/// Ask the UI thread to place text on the system clipboard (received text).
/// The UI thread owns the message loop, which the clipboard API requires.
/// LPARAM -> *mut String  (the UTF-16 text to copy)
pub const WM_COPY_CLIPBOARD_TEXT: u32 = WM_USER_BASE + 11;

/// Instruct a window to re-read the active language pack and re-localise all of
/// its visible text. Posted to the main window, which forwards it to open
/// children (receive / text dialogs) after localising itself.
pub const WM_LANG_CHANGED: u32 = WM_USER_BASE + 12;

/// Payload for [`WM_TRANSFER_REQUEST`].
pub struct IncomingTransfer {
    pub session_id: String,
    pub sender: DeviceInfo,
    pub files: Vec<FileMeta>,
    /// Channel used by the UI to tell the server whether to accept.
    pub decision_tx: watch::Sender<TransferDecision>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransferDecision {
    Pending,
    Accepted,
    Rejected,
}

/// Payload for [`WM_TRANSFER_PROGRESS`].
pub struct ProgressUpdate {
    pub session_id: String,
    pub file_name: String,
    pub bytes: u64,
    pub total: u64,
    /// true = outgoing (we are sending), false = incoming.
    pub outgoing: bool,
}

/// Payload for [`WM_TRANSFER_COMPLETE`].
pub struct TransferComplete {
    pub session_id: String,
    pub file_count: usize,
    /// Destination folder (incoming) or empty (outgoing).
    pub dest_dir: String,
    pub outgoing: bool,
    pub peer_alias: String,
    /// true when this was a text (clipboard) transfer rather than files.
    pub is_text: bool,
}

/// Payload for [`WM_TRANSFER_FAILED`].
pub struct TransferFailed {
    /// Session identifier, retained for diagnostics/logging but not currently read.
    pub _session_id: String,
    pub reason: String,
    pub outgoing: bool,
}
