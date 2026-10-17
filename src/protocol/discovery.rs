//! LAN discovery for LocalSend v2.2.
//!
//! Default method: multicast UDP. We periodically announce ourselves to the
//! group `224.0.0.167:53317` and listen for peers' announcements. When we hear
//! a peer we (a) record it and (b) register with it over HTTP so it learns about
//! us too. Incoming `/register` requests (handled in `server.rs`) complete the
//! two-way discovery.

use crate::protocol::types::{Announce, DeviceInfo};
use crate::AppInner;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::net::UdpSocket;

const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const PROTOCOL_VERSION: &str = "2.0";

/// Best-effort primary outbound IPv4 interface.
fn local_ip() -> Ipv4Addr {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok();
    if let Some(s) = sock {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = s.local_addr() {
                if let SocketAddr::V4(v4) = addr {
                    return *v4.ip();
                }
            }
        }
    }
    Ipv4Addr::UNSPECIFIED
}

/// Entry point spawned on the tokio runtime.
pub async fn run_discovery(app: Arc<AppInner>) {
    let multicast_port = { app.config.lock().unwrap().multicast_port };
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, multicast_port);

    let recv_sock = match UdpSocket::bind(bind_addr).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("discovery: cannot bind UDP {bind_addr}: {e}");
            return;
        }
    };

    let iface = local_ip();
    // Joining may fail on some adapters; log but keep trying to send.
    if let Err(e) = recv_sock.join_multicast_v4(MULTICAST_ADDR, iface) {
        log::warn!("discovery: join_multicast_v4 failed ({e}); receive may not work");
    }

    // Sender socket (ephemeral) for announcements.
    let send_sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            log::error!("discovery: cannot bind sender UDP: {e}");
            return;
        }
    };
    let _ = send_sock.set_multicast_loop_v4(false);

    // Announce immediately, then every 3s.
    let app_send = app.clone();
    tokio::spawn(async move {
        let dest = SocketAddrV4::new(MULTICAST_ADDR, multicast_port);
        loop {
            if let Err(e) = send_announce(&app_send, &send_sock, dest).await {
                log::debug!("discovery send: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });

    // Receive loop.
    let mut buf = vec![0u8; 4096];
    loop {
        match recv_sock.recv_from(&mut buf).await {
            Ok((n, src)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                if let Ok(ann) = serde_json::from_str::<Announce>(&text) {
                    // Avoid discovering ourselves.
                    if ann.fingerprint == app.fingerprint {
                        continue;
                    }
                    let peer_ip = match src {
                        SocketAddr::V4(v4) => *v4.ip(),
                        SocketAddr::V6(_) => continue,
                    };
                    let info = DeviceInfo::from_announce(&ann, peer_ip.to_string());
                    app.upsert_device(info.clone());
                    // Respond over HTTP so the peer learns about us.
                    let app2 = app.clone();
                    tokio::spawn(async move {
                        let _ = register_with_peer(&app2, &info).await;
                    });
                }
            }
            Err(e) => {
                log::debug!("discovery recv: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

async fn send_announce(
    app: &Arc<AppInner>,
    sock: &UdpSocket,
    dest: SocketAddrV4,
) -> std::io::Result<()> {
    // Scope the lock tightly: clone the needed config values, then drop the
    // guard before any `.await` so the future stays `Send`.
    let (alias, device_model, device_type, port, require_encryption) = {
        let cfg = app.config.lock().unwrap();
        (
            cfg.alias.clone(),
            cfg.device_model.clone(),
            cfg.device_type.clone(),
            cfg.port,
            cfg.require_encryption,
        )
    };
    let announce = Announce {
        alias,
        version: PROTOCOL_VERSION.to_string(),
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
        announce: true,
    };
    let payload = serde_json::to_vec(&announce)?;
    sock.send_to(&payload, dest).await?;
    Ok(())
}

/// POST /api/localsend/v2/register to a peer so they add us to their list.
async fn register_with_peer(app: &Arc<AppInner>, peer: &DeviceInfo) -> anyhow::Result<()> {
    let (alias, device_model, device_type, port, require_encryption) = {
        let cfg = app.config.lock().unwrap();
        (
            cfg.alias.clone(),
            cfg.device_model.clone(),
            cfg.device_type.clone(),
            cfg.port,
            cfg.require_encryption,
        )
    };
    let body = Announce {
        alias,
        version: PROTOCOL_VERSION.to_string(),
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
    let client = app.http_client();
    let url = format!("{}/api/localsend/v2/register", peer.base_url());
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await?;
    if resp.status().is_success() {
        log::debug!("registered with {}", peer.alias);
    } else {
        log::debug!("register {} -> {}", peer.alias, resp.status());
    }
    Ok(())
}

/// Convenience used by `server.rs` when handling `/register`.
pub fn device_from_announce(ann: &Announce, ip: String) -> DeviceInfo {
    DeviceInfo::from_announce(ann, ip)
}
