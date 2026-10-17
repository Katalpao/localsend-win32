//! Sending side: acts as the LocalSend HTTP client.
//!
//! Given a target device and a list of local files, it performs
//! `prepare-upload` then streams each file to `/upload`. Progress and
//! completion are reported back to the UI thread.

use crate::msg::{
    ProgressUpdate, TransferComplete, TransferFailed, WM_TRANSFER_COMPLETE, WM_TRANSFER_FAILED,
    WM_TRANSFER_PROGRESS,
};
use crate::protocol::types::*;
use crate::AppInner;
use anyhow::Result;
use bytes::Bytes;
use futures_util::stream::unfold;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

/// Kick off a send to `target`. Runs on the tokio runtime.
pub async fn send_files(app: Arc<AppInner>, target: DeviceInfo, files: Vec<PathBuf>) {
    let session = Uuid::new_v4().to_string();
    match do_send(app.clone(), target.clone(), files.clone(), session.clone()).await {
        Ok(()) => {
            let comp = TransferComplete {
                session_id: session,
                file_count: files.len(),
                dest_dir: String::new(),
                outgoing: true,
                peer_alias: target.alias.clone(),
                is_text: false,
            };
            app.post(WM_TRANSFER_COMPLETE, Box::into_raw(Box::new(comp)) as isize);
            // Remember last target for "quick send".
            {
                let mut c = app.config.lock().unwrap();
                c.last_target_id = Some(target.id());
                let _ = c.save();
            }
        }
        Err(e) => {
            let fail = TransferFailed {
                _session_id: session,
                reason: e.to_string(),
                outgoing: true,
            };
            app.post(WM_TRANSFER_FAILED, Box::into_raw(Box::new(fail)) as isize);
        }
    }
}

/// Kick off a text send to `target`. The text is delivered as a LocalSend
/// `text/plain` transfer so the official client copies it straight to the
/// clipboard (and our own receiver does the same).
pub async fn send_text(app: Arc<AppInner>, target: DeviceInfo, text: String) {
    let session = Uuid::new_v4().to_string();
    match do_send_text(app.clone(), target.clone(), text, session.clone()).await {
        Ok(()) => {
            let comp = TransferComplete {
                session_id: session,
                file_count: 1,
                dest_dir: String::new(),
                outgoing: true,
                peer_alias: target.alias.clone(),
                is_text: true,
            };
            app.post(WM_TRANSFER_COMPLETE, Box::into_raw(Box::new(comp)) as isize);
            {
                let mut c = app.config.lock().unwrap();
                c.last_target_id = Some(target.id());
                let _ = c.save();
            }
        }
        Err(e) => {
            let fail = TransferFailed {
                _session_id: session,
                reason: e.to_string(),
                outgoing: true,
            };
            app.post(WM_TRANSFER_FAILED, Box::into_raw(Box::new(fail)) as isize);
        }
    }
}

async fn do_send_text(
    app: Arc<AppInner>,
    target: DeviceInfo,
    text: String,
    _session_id: String,
) -> Result<()> {
    let bytes = text.clone().into_bytes();
    let size = bytes.len() as u64;
    let id = Uuid::new_v4().to_string();

    // SHA-256 of the payload, matching what the official client sends (optional
    // but improves compatibility / quick-save behaviour on the receiver).
    let digest = Sha256::digest(&bytes);
    let hash_hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();

    // Official text messages are sent as a single text/plain file with a
    // random `.txt` fileName and the message itself in the `preview` field.
    // The receiver identifies it as a "message request" when there is exactly
    // one file, `fileType` decodes to `FileType.text`, and `preview` is set.
    // Without `preview` the receiver treats the transfer as a plain file save.
    let file_name = format!("{}.txt", Uuid::new_v4());
    let mut file_map: HashMap<String, FileMeta> = HashMap::new();
    file_map.insert(
        id.clone(),
        FileMeta {
            id: id.clone(),
            file_name: file_name.clone(),
            size,
            file_type: Some("text/plain".to_string()),
            sha256: Some(hash_hex),
            preview: Some(text),
            metadata: None,
        },
    );

    // Scope the lock tightly so the `MutexGuard` is dropped before any await.
    let (alias, device_model, device_type, port, require_encryption, pin) = {
        let cfg = app.config.lock().unwrap();
        (
            cfg.alias.clone(),
            cfg.device_model.clone(),
            cfg.device_type.clone(),
            cfg.port,
            cfg.require_encryption,
            cfg.pin.clone(),
        )
    };
    let announce = Announce {
        alias,
        version: "2.0".to_string(),
        device_model: Some(device_model),
        device_type: Some(device_type),
        fingerprint: app.fingerprint.clone(),
        port,
        protocol: if require_encryption {
            "https".to_string()
        } else {
            "http".to_string()
        },
        download: false,
        announce: false,
    };

    let req = PrepareUploadRequest {
        info: announce,
        files: file_map.clone(),
    };

    let client = app.http_client();
    let url = if pin.is_empty() {
        format!("{}/api/localsend/v2/prepare-upload", target.base_url())
    } else {
        format!(
            "{}/api/localsend/v2/prepare-upload?pin={}",
            target.base_url(),
            pin
        )
    };
    let resp = client.post(&url).json(&req).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("prepare-upload failed: {}", resp.status());
    }
    let prep: PrepareUploadResponse = resp.json().await?;
    let session_id = prep.session_id.clone();
    let token = prep.files.get(&id).cloned().unwrap_or_default();

    upload_text(&app, &target, &session_id, &id, &token, bytes, &file_name, size).await?;
    Ok(())
}

async fn upload_text(
    app: &Arc<AppInner>,
    target: &DeviceInfo,
    session_id: &str,
    file_id: &str,
    token: &str,
    data: Vec<u8>,
    file_name: &str,
    total: u64,
) -> Result<()> {
    let client = app.http_client();
    let url = format!(
        "{}/api/localsend/v2/upload?sessionId={}&fileId={}&token={}",
        target.base_url(),
        session_id,
        file_id,
        token
    );
    let resp = client.post(&url).body(data).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("upload {} -> {}", file_name, resp.status());
    }
    let upd = ProgressUpdate {
        session_id: session_id.to_string(),
        file_name: file_name.to_string(),
        bytes: total,
        total,
        outgoing: true,
    };
    app.post(WM_TRANSFER_PROGRESS, Box::into_raw(Box::new(upd)) as isize);
    Ok(())
}

async fn do_send(
    app: Arc<AppInner>,
    target: DeviceInfo,
    files: Vec<PathBuf>,
    _session_id: String,
) -> Result<()> {
    // Build metadata.
    let mut file_map: HashMap<String, FileMeta> = HashMap::new();
    for path in &files {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.bin")
            .to_string();
        let size = tokio::fs::metadata(path).await?.len();
        let id = Uuid::new_v4().to_string();
        file_map.insert(
            id.clone(),
            FileMeta {
                id: id.clone(),
                file_name: name,
                size,
                file_type: Some(guess_mime(path)),
                sha256: None,
                preview: None,
                metadata: None,
            },
        );
    }

    // Scope the lock tightly so the `MutexGuard` is dropped before any await.
    let (alias, device_model, device_type, port, require_encryption, pin) = {
        let cfg = app.config.lock().unwrap();
        (
            cfg.alias.clone(),
            cfg.device_model.clone(),
            cfg.device_type.clone(),
            cfg.port,
            cfg.require_encryption,
            cfg.pin.clone(),
        )
    };
    let announce = Announce {
        alias,
        version: "2.0".to_string(),
        device_model: Some(device_model),
        device_type: Some(device_type),
        fingerprint: app.fingerprint.clone(),
        port,
        protocol: if require_encryption {
            "https".to_string()
        } else {
            "http".to_string()
        },
        download: false,
        announce: false,
    };

    let req = PrepareUploadRequest {
        info: announce,
        files: file_map.clone(),
    };

    let client = app.http_client();
    let url = if pin.is_empty() {
        format!("{}/api/localsend/v2/prepare-upload", target.base_url())
    } else {
        format!(
            "{}/api/localsend/v2/prepare-upload?pin={}",
            target.base_url(),
            pin
        )
    };
    let resp = client.post(&url).json(&req).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("prepare-upload failed: {}", resp.status());
    }
    let prep: PrepareUploadResponse = resp.json().await?;
    let session_id = prep.session_id.clone();

    // Upload each file. We run them concurrently; progress is posted as we go.
    let completed = Arc::new(AtomicUsize::new(0));
    let total = file_map.len();
    let mut handles = Vec::new();
    for (id, meta) in &file_map {
        let token = prep.files.get(id).cloned().unwrap_or_default();
        let path = files
            .iter()
            .find(|p| {
                p.file_name().and_then(|s| s.to_str()) == Some(meta.file_name.as_str())
            })
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&meta.file_name));
        let app2 = app.clone();
        let target2 = target.clone();
        let sid = session_id.clone();
        let fid = id.clone();
        let tok = token.clone();
        let fname = meta.file_name.clone();
        let size = meta.size;
        let comp2 = completed.clone();
        handles.push(tokio::spawn(async move {
            let r = upload_one(&app2, &target2, &sid, &fid, &tok, &path, &fname, size).await;
            comp2.fetch_add(1, Ordering::Relaxed);
            r
        }));
    }

    let mut last_err = None;
    for h in handles {
        if let Ok(Err(e)) = h.await {
            last_err = Some(e);
        }
    }
    if let Some(e) = last_err {
        anyhow::bail!("upload failed: {e}");
    }
    let _ = (session_id, total);
    Ok(())
}

async fn upload_one(
    app: &Arc<AppInner>,
    target: &DeviceInfo,
    session_id: &str,
    file_id: &str,
    token: &str,
    path: &std::path::Path,
    file_name: &str,
    total: u64,
) -> Result<()> {
    let client = app.http_client();
    let _url = format!("{}/api/localsend/v2/upload", target.base_url());
    let file = tokio::fs::File::open(path).await?;

    // Stream the file in chunks, reporting progress to the UI as we go.
    // `futures_util::stream::unfold` lets us read the file incrementally while
    // keeping the stream `Send` (so it can drive `reqwest::Body::wrap_stream`).
    let app2 = app.clone();
    let sid = session_id.to_string();
    let fname = file_name.to_string();
    let stream = unfold((file, 0u64), move |(mut file, sent)| {
        // `unfold` calls this `FnMut` closure once per chunk, so the captured
        // values must be cloned (not moved) into each generated future.
        let app2 = app2.clone();
        let sid = sid.clone();
        let fname = fname.clone();
        async move {
            let mut buf = vec![0u8; 64 * 1024];
            match file.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    let sent = sent + n as u64;
                    let upd = ProgressUpdate {
                        session_id: sid.clone(),
                        file_name: fname.clone(),
                        bytes: sent,
                        total,
                        outgoing: true,
                    };
                    app2.post(WM_TRANSFER_PROGRESS, Box::into_raw(Box::new(upd)) as isize);
                    Some((
                        Ok::<_, std::io::Error>(Bytes::copy_from_slice(&buf[..n])),
                        (file, sent),
                    ))
                }
                Err(e) => Some((Err(e), (file, sent))),
            }
        }
    });
    let body = reqwest::Body::wrap_stream(stream);

    let url = format!(
        "{}/api/localsend/v2/upload?sessionId={}&fileId={}&token={}",
        target.base_url(),
        session_id,
        file_id,
        token
    );
    let resp = client.post(&url).body(body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("upload {} -> {}", file_name, resp.status());
    }
    // Final progress marker.
    let upd = ProgressUpdate {
        session_id: session_id.to_string(),
        file_name: file_name.to_string(),
        bytes: total,
        total,
        outgoing: true,
    };
    app.post(WM_TRANSFER_PROGRESS, Box::into_raw(Box::new(upd)) as isize);
    Ok(())
}

fn guess_mime(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        "ppt" | "pptx" => "application/vnd.ms-powerpoint",
        _ => "application/octet-stream",
    }
    .to_string()
}
