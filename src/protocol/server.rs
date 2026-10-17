//! Receiving side: the LocalSend HTTP server.
//!
//! Implements (LocalSend v2.2):
//!   GET  /api/localsend/v2/info
//!   POST /api/localsend/v2/register        (two-way discovery)
//!   POST /api/localsend/v2/prepare-upload  (metadata, returns session + tokens)
//!   POST /api/localsend/v2/upload          (raw binary body per file)
//!   POST /api/localsend/v2/cancel
//!
//! When a `prepare-upload` arrives we create a session, post a
//! `WM_TRANSFER_REQUEST` to the UI so the user can accept/reject, and let the
//! `upload` handlers stream files once accepted.

use crate::msg::{
    ProgressUpdate, TransferComplete, TransferDecision, WM_COPY_CLIPBOARD_TEXT,
    WM_TRANSFER_COMPLETE, WM_TRANSFER_PROGRESS, WM_TRANSFER_REQUEST,
};
use crate::protocol::discovery::device_from_announce;
use crate::protocol::types::*;
use crate::AppInner;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

type BoxErr = (StatusCode, String);

#[derive(Clone)]
pub struct ServerState {
    pub app: Arc<AppInner>,
    pub sessions: Arc<Mutex<HashMap<String, Session>>>,
}

/// One incoming transfer, kept alive while files stream in.
pub struct Session {
    pub sender: DeviceInfo,
    pub files: HashMap<String, FileMeta>,
    pub tokens: HashMap<String, String>,
    /// UI flips this to Accepted/Rejected.
    pub decision: tokio::sync::watch::Sender<TransferDecision>,
    pub dest_dir: PathBuf,
    pub written: Arc<AtomicUsize>,
    pub total_files: usize,
    /// Accumulated text for `fileType == "text"` entries (copied to clipboard
    /// on completion instead of being written to disk).
    pub text: Arc<Mutex<String>>,
}

#[derive(serde::Deserialize)]
struct PinQuery {
    pin: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    session_id: String,
    file_id: String,
    token: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelQuery {
    session_id: String,
}

/// Build the axum router bound to `app`.
pub fn build_router(app: Arc<AppInner>) -> Router {
    let state = ServerState {
        app,
        sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/api/localsend/v2/info", get(info))
        .route("/api/localsend/v2/register", post(register))
        .route("/api/localsend/v2/prepare-upload", post(prepare_upload))
        .route("/api/localsend/v2/upload", post(upload))
        .route("/api/localsend/v2/cancel", post(cancel))
        .with_state(state)
}

/// Run the server (plain HTTP or, with the `https` feature, TLS).
pub async fn run_server(app: Arc<AppInner>) -> anyhow::Result<()> {
    let (port, _encryption) = {
        let c = app.config.lock().unwrap();
        (c.port, c.require_encryption)
    };
    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    let router = build_router(app.clone());

    #[cfg(feature = "https")]
    {
        if encryption {
            return run_server_tls(app, addr, router).await;
        }
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("HTTP server listening on {addr}");
    let make_service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    axum::serve(listener, make_service).await?;
    Ok(())
}

#[cfg(feature = "https")]
async fn run_server_tls(
    _app: Arc<AppInner>,
    addr: SocketAddrV4,
    router: Router,
) -> anyhow::Result<()> {
    use axum_server::tls_rustls::RustlsConfig;
    use rcgen::{CertificateParams, KeyPair};
    use rustls::ServerConfig;

    let mut params = CertificateParams::new(vec!["localhost".to_string()]);
    params.distinguished_name = Default::default();
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    let cert_der = cert.serialize_der()?;
    let key_der = key.serialize_der();

    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::Certificate(cert_der)],
            rustls::PrivateKey(key_der),
        )?;
    let rustls_config = RustlsConfig::from_config(Arc::new(server_config));
    log::info!("HTTPS server listening on {addr}");
    axum_server::Server::bind(addr).tls_config(rustls_config).serve(router.into_make_service()).await?;
    Ok(())
}

// ---- handlers -----------------------------------------------------------

async fn info(State(st): State<ServerState>) -> Json<InfoResponse> {
    let c = st.app.config.lock().unwrap();
    Json(InfoResponse {
        alias: c.alias.clone(),
        version: "2.0".to_string(),
        device_model: Some(c.device_model.clone()),
        device_type: Some(c.device_type.clone()),
        fingerprint: st.app.fingerprint.clone(),
        download: false,
    })
}

async fn register(
    State(st): State<ServerState>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<Announce>,
) -> Json<Announce> {
    let ip = addr.ip().to_string();
    let info = device_from_announce(&body, ip);
    st.app.upsert_device(info);
    let c = st.app.config.lock().unwrap();
    Json(Announce {
        alias: c.alias.clone(),
        version: "2.0".to_string(),
        device_model: Some(c.device_model.clone()),
        device_type: Some(c.device_type.clone()),
        fingerprint: st.app.fingerprint.clone(),
        port: c.port,
        protocol: if c.require_encryption {
            "https".to_string()
        } else {
            "http".to_string()
        },
        download: false,
        announce: false,
    })
}

async fn prepare_upload(
    State(st): State<ServerState>,
    Query(q): Query<PinQuery>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<PrepareUploadRequest>,
) -> Result<Json<PrepareUploadResponse>, BoxErr> {
    // PIN check.
    let cfg_pin = { st.app.config.lock().unwrap().pin.clone() };
    if !cfg_pin.is_empty() {
        if q.pin.as_deref() != Some(cfg_pin.as_str()) {
            return Err((StatusCode::UNAUTHORIZED, "PIN required / invalid".into()));
        }
    }

    let session_id = Uuid::new_v4().to_string();
    let mut tokens = HashMap::new();
    for (id, _f) in &req.files {
        tokens.insert(id.clone(), Uuid::new_v4().to_string());
    }
    let (decision_tx, _rx) = tokio::sync::watch::channel(TransferDecision::Pending);

    let dest_dir = { st.app.config.lock().unwrap().download_dir.clone() };
    let _ = tokio::fs::create_dir_all(&dest_dir).await;

    let sender = DeviceInfo::from_announce(&req.info, addr.ip().to_string());

    let session = Session {
        sender: sender.clone(),
        files: req.files.clone(),
        tokens: tokens.clone(),
        decision: decision_tx.clone(),
        dest_dir: dest_dir.clone(),
        written: Arc::new(AtomicUsize::new(0)),
        total_files: req.files.len(),
        text: Arc::new(Mutex::new(String::new())),
    };
    st.sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);

    // Ask the UI to confirm.
    let incoming = crate::msg::IncomingTransfer {
        session_id: session_id.clone(),
        sender,
        files: req.files.values().cloned().collect(),
        decision_tx,
    };
    st.app.post(WM_TRANSFER_REQUEST, Box::into_raw(Box::new(incoming)) as isize);

    Ok(Json(PrepareUploadResponse {
        session_id,
        files: tokens,
    }))
}

async fn upload(
    State(st): State<ServerState>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<StatusCode, BoxErr> {
    let session = {
        let map = st.sessions.lock().unwrap();
        match map.get(&q.session_id) {
            Some(s) => s.clone_light(),
            None => return Err((StatusCode::NOT_FOUND, "unknown session".into())),
        }
    };

    // Token check.
    let expected = session.tokens.get(&q.file_id).cloned();
    if expected.as_deref() != Some(q.token.as_str()) {
        return Err((StatusCode::FORBIDDEN, "invalid token".into()));
    }

    // Wait for the user's decision.
    let decided = wait_for_decision(&session.decision).await;
    if decided != TransferDecision::Accepted {
        return Err((StatusCode::FORBIDDEN, "rejected".into()));
    }

    let meta = session
        .files
        .get(&q.file_id)
        .cloned()
        .ok_or((StatusCode::BAD_REQUEST, "unknown file".into()))?;

    // --- Text transfer: copy the content to the clipboard, do NOT write a file. ---
    // The official client serializes `fileType` as a MIME string (e.g.
    // "text/plain") via `lookupMime(fileName)`, NOT the legacy enum name
    // "text". Mirror the official `decode`: text if the value is the bare
    // "text" (legacy) or any "text/..." MIME.
    let is_text = meta
        .file_type
        .as_deref()
        .map(|t| t == "text" || t.starts_with("text/"))
        .unwrap_or(false);
    log::info!(
        "upload file_id={} file_name={:?} file_type={:?} -> treating as text={}",
        q.file_id,
        meta.file_name,
        meta.file_type,
        is_text
    );
    if is_text {
        let text = String::from_utf8_lossy(body.as_ref()).into_owned();
        {
            let mut buf = session.text.lock().unwrap();
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&text);
        }
        let upd = ProgressUpdate {
            session_id: q.session_id.clone(),
            file_name: meta.file_name.clone(),
            bytes: meta.size,
            total: meta.size,
            outgoing: false,
        };
        st.app.post(WM_TRANSFER_PROGRESS, Box::into_raw(Box::new(upd)) as isize);
        finish_upload(&st, &session, &q.session_id);
        return Ok(StatusCode::OK);
    }

    // --- File transfer: stream the bytes to disk. ---
    let _total = meta.size;

    // Resolve a safe destination path.
    let file_name = sanitize_name(&meta.file_name);
    let dest = unique_path(&session.dest_dir, &file_name);
    let mut file = tokio::fs::File::create(&dest)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let sid = q.session_id.clone();
    let fname = file_name.clone();
    let total = meta.size;
    let mut offset: usize = 0;
    for chunk in body.as_ref().chunks(256 * 1024) {
        file.write_all(chunk)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        offset += chunk.len();
        let upd = ProgressUpdate {
            session_id: sid.clone(),
            file_name: fname.clone(),
            bytes: offset as u64,
            total,
            outgoing: false,
        };
        st.app.post(WM_TRANSFER_PROGRESS, Box::into_raw(Box::new(upd)) as isize);
    }
    file.flush().await.ok();

    finish_upload(&st, &session, &q.session_id);
    Ok(StatusCode::OK)
}

/// Account for one finished file (text or binary). When every file in the
/// session is done, either copy accumulated text to the clipboard or just
/// signal completion, then remove the session.
fn finish_upload(st: &ServerState, session: &SessionLight, session_id: &str) {
    let done = session.written.fetch_add(1, Ordering::Relaxed) + 1;
    if done < session.total_files {
        return;
    }
    let text = session.text.lock().unwrap().clone();
    let is_text = !text.is_empty();
    if is_text {
        // Hand the text to the UI thread; clipboard ops require a message loop.
        st.app.post(
            WM_COPY_CLIPBOARD_TEXT,
            Box::into_raw(Box::new(text)) as isize,
        );
    }
    let comp = TransferComplete {
        session_id: session_id.to_string(),
        file_count: session.total_files,
        dest_dir: session.dest_dir.to_string_lossy().to_string(),
        outgoing: false,
        peer_alias: session.sender.alias.clone(),
        is_text,
    };
    st.app.post(WM_TRANSFER_COMPLETE, Box::into_raw(Box::new(comp)) as isize);
    st.sessions.lock().unwrap().remove(session_id);
}

async fn cancel(
    State(st): State<ServerState>,
    Query(q): Query<CancelQuery>,
) -> StatusCode {
    st.sessions.lock().unwrap().remove(&q.session_id);
    StatusCode::OK
}

// ---- helpers ------------------------------------------------------------

async fn wait_for_decision(tx: &tokio::sync::watch::Sender<TransferDecision>) -> TransferDecision {
    let mut rx = tx.subscribe();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if *rx.borrow() != TransferDecision::Pending {
            return *rx.borrow();
        }
        match tokio::time::timeout_at(deadline, rx.changed()).await {
            Ok(Ok(())) => continue,
            _ => return TransferDecision::Rejected, // timed out
        }
    }
}

fn sanitize_name(name: &str) -> String {
    let p = std::path::Path::new(name);
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "file.bin".to_string())
}

fn unique_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    for i in 1..1000 {
        let cand = if ext.is_empty() {
            dir.join(format!("{stem} ({i})"))
        } else {
            dir.join(format!("{stem} ({i}).{ext}"))
        };
        if !cand.exists() {
            return cand;
        }
    }
    dir.join(format!("{stem}-{}", Uuid::new_v4()))
}

impl Session {
    /// Clone only the fields needed by an upload handler (avoids holding the
    /// decision sender longer than necessary).
    fn clone_light(&self) -> SessionLight {
        SessionLight {
            sender: self.sender.clone(),
            files: self.files.clone(),
            tokens: self.tokens.clone(),
            decision: self.decision.clone(),
            dest_dir: self.dest_dir.clone(),
            written: self.written.clone(),
            total_files: self.total_files,
            text: self.text.clone(),
        }
    }
}

/// Lightweight clone of a session for per-upload handling.
struct SessionLight {
    sender: DeviceInfo,
    files: HashMap<String, FileMeta>,
    tokens: HashMap<String, String>,
    decision: tokio::sync::watch::Sender<TransferDecision>,
    dest_dir: PathBuf,
    written: Arc<AtomicUsize>,
    total_files: usize,
    text: Arc<Mutex<String>>,
}
