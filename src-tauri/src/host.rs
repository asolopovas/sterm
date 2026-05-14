use crate::{AppState, HostServiceInfo, PairingPayload, TerminalShell};
use anyhow::{Context, Result};
use igd::{aio::search_gateway, PortMappingProtocol, SearchOptions};
use quinn::Endpoint;
use rterm_protocol::config;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    sync::Arc,
};
use tauri::Emitter;
use tokio::sync::Semaphore;

const DEFAULT_LISTEN_PORT: u16 = 4433;
const DEFAULT_INTERNET_TRACKER: &str = "udp://tracker.opentrackr.org:1337";
const DEFAULT_TRACKER_ROOM: &str = "sterm";

pub async fn start_host_service(
    app: tauri::AppHandle,
    state: AppState,
    token: String,
    first_run: bool,
) {
    if let Err(err) = run_host_service(app.clone(), state.clone(), token, first_run).await {
        state.set_host_error(err.to_string());
        let _ = app.emit("service-status", format!("host service failed: {err:#}"));
    }
}

async fn run_host_service(
    app: tauri::AppHandle,
    state: AppState,
    token: String,
    first_run: bool,
) -> Result<()> {
    let (endpoint, certificate_sha256_pin) = bind_endpoint(DEFAULT_LISTEN_PORT)?;

    let listen = endpoint.local_addr()?;
    let lan_ip = lan_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let lan_addr = SocketAddr::new(lan_ip, listen.port());
    let upnp_attempt = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        add_upnp_mapping(listen, listen.port(), config::DEFAULT_UPNP_LEASE_SECONDS),
    )
    .await;
    let upnp_mapping = match upnp_attempt {
        Ok(Ok(mapping)) => {
            let _ = app.emit(
                "service-status",
                format!(
                    "UPnP mapped {}:{}",
                    mapping.external_ip, mapping.external_port
                ),
            );
            Some(mapping)
        }
        Ok(Err(err)) => {
            let _ = app.emit(
                "service-status",
                format!("UPnP unavailable; tracker P2P may need manual UDP forwarding: {err:#}"),
            );
            None
        }
        Err(_) => {
            let _ = app.emit(
                "service-status",
                "UPnP timed out; tracker P2P may need manual UDP forwarding",
            );
            None
        }
    };
    let stun_public_addr = if upnp_mapping.is_none() && listen.port() == DEFAULT_LISTEN_PORT {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::p2p_probe::discover_public_ip_via_stun(),
        )
        .await
        {
            Ok(Ok(ip)) => {
                let addr = SocketAddr::new(ip, listen.port());
                let _ = app.emit(
                    "service-status",
                    format!("STUN found public address candidate {addr}"),
                );
                Some(addr)
            }
            Ok(Err(err)) => {
                let _ = app.emit(
                    "service-status",
                    format!("STUN public IP lookup failed: {err:#}"),
                );
                None
            }
            Err(_) => {
                let _ = app.emit("service-status", "STUN public IP lookup timed out");
                None
            }
        }
    } else {
        None
    };
    let public_addr = upnp_mapping
        .as_ref()
        .map(|mapping| SocketAddr::new(IpAddr::V4(mapping.external_ip), mapping.external_port))
        .or(stun_public_addr);
    let pairing = PairingPayload {
        v: 1,
        mode: "tracker".to_string(),
        host: public_addr.or(Some(lan_addr)).map(|addr| addr.to_string()),
        token: token.clone(),
        cert_sha256: certificate_sha256_pin.clone(),
        tracker: Some(DEFAULT_INTERNET_TRACKER.to_string()),
        tracker_room: Some(DEFAULT_TRACKER_ROOM.to_string()),
        relay: None,
        relay_cert_sha256: None,
        requires_password: Some(
            state
                .host_settings()
                .map(|settings| settings.password.is_some())
                .unwrap_or(false),
        ),
        password: None,
    };
    let settings = state.host_settings().map_err(anyhow::Error::msg)?;
    let pairing_json = compact_pairing_json(&pairing)?;
    let qr_svg = render_qr_svg(&pairing_json)?;

    state.set_host_info(HostServiceInfo {
        platform: "desktop".to_string(),
        running: true,
        status: format!("internet P2P via {DEFAULT_INTERNET_TRACKER}"),
        listen: listen.to_string(),
        lan_address: Some(lan_addr.to_string()),
        cert_sha256: certificate_sha256_pin,
        first_run,
        shell: settings.shell,
        password_enabled: settings.password.is_some(),
        pairing,
        pairing_json,
        qr_svg,
    });
    let _ = app.emit(
        "service-status",
        "host service listening with tracker P2P discovery",
    );

    {
        let app = app.clone();
        let token = token.clone();
        tokio::spawn(async move {
            announce_tracker_loop(
                app,
                DEFAULT_INTERNET_TRACKER.to_string(),
                DEFAULT_TRACKER_ROOM.to_string(),
                token,
                listen.port(),
            )
            .await;
        });
    }

    let _upnp_mapping = upnp_mapping;
    let connection_limit = Arc::new(Semaphore::new(config::DIRECT_HOST_CONNECTION_LIMIT));

    while let Some(connecting) = endpoint.accept().await {
        let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
            let _ = app.emit(
                "service-status",
                "connection limit reached; dropping client",
            );
            continue;
        };
        let token = token.clone();
        let app = app.clone();
        let settings = match state.host_settings().map_err(anyhow::Error::msg) {
            Ok(settings) => settings,
            Err(err) => {
                let _ = app.emit(
                    "service-status",
                    format!("host settings unavailable: {err:#}"),
                );
                continue;
            }
        };
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(err) = handle_connection(connecting, settings, token).await {
                let _ = app.emit("service-status", format!("connection ended: {err:#}"));
            } else {
                let _ = app.emit("service-status", "client session ended");
            }
        });
    }

    Ok(())
}

pub fn enable_tracker_pairing(
    app: tauri::AppHandle,
    state: AppState,
    tracker: String,
    room: String,
    already_running: bool,
) -> Result<HostServiceInfo> {
    let mut info = state.host_info().map_err(anyhow::Error::msg)?;
    info.pairing.mode = "tracker".to_string();
    info.pairing.tracker = Some(tracker.clone());
    info.pairing.tracker_room = Some(room.clone());
    info.pairing.relay = None;
    info.pairing.relay_cert_sha256 = None;
    info.pairing_json = compact_pairing_json(&info.pairing)?;
    info.qr_svg = render_qr_svg(&info.pairing_json)?;
    info.status = format!("tracker P2P via {tracker}");
    state.set_host_info(info.clone());

    if !already_running {
        let token = info.pairing.token.clone();
        let listen_port = parse_listen_port(&info.listen)?;
        tokio::spawn(async move {
            announce_tracker_loop(app, tracker, room, token, listen_port).await;
        });
    }

    Ok(info)
}

async fn announce_tracker_loop(
    app: tauri::AppHandle,
    tracker: String,
    room: String,
    token: String,
    listen_port: u16,
) {
    let resolved = match rterm_protocol::resolve_tracker(&tracker) {
        Ok(resolved) => resolved,
        Err(err) => {
            let _ = app.emit("service-status", format!("tracker resolve failed: {err:#}"));
            return;
        }
    };

    loop {
        match rterm_protocol::announce_to_tracker(&resolved, token.as_bytes(), &room, listen_port)
            .await
        {
            Ok(announce) => {
                let _ = app.emit(
                    "service-status",
                    format!(
                        "announced to tracker {}; {} peer(s), next in {:?}",
                        resolved.addr,
                        announce.peers.len(),
                        announce.interval
                    ),
                );
                tokio::time::sleep(announce.interval).await;
            }
            Err(err) => {
                let _ = app.emit(
                    "service-status",
                    format!("tracker announce failed: {err:#}"),
                );
                tokio::time::sleep(config::TRACKER_RETRY_DELAY).await;
            }
        }
    }
}

pub fn enable_relay_pairing(
    app: tauri::AppHandle,
    state: AppState,
    relay: String,
    relay_cert_sha256: Option<String>,
    already_running: bool,
) -> Result<HostServiceInfo> {
    let mut info = state.host_info().map_err(anyhow::Error::msg)?;
    info.pairing.mode = "relay".to_string();
    info.pairing.host = None;
    info.pairing.tracker = None;
    info.pairing.tracker_room = None;
    info.pairing.relay = Some(relay.clone());
    info.pairing.relay_cert_sha256 = relay_cert_sha256.clone();
    info.pairing_json = compact_pairing_json(&info.pairing)?;
    info.qr_svg = render_qr_svg(&info.pairing_json)?;
    info.status = format!("relay rendezvous via {relay}");
    state.set_host_info(info.clone());

    if !already_running {
        let token = info.pairing.token.clone();
        tokio::spawn(async move {
            run_relay_host_loop(app, relay, token, relay_cert_sha256).await;
        });
    }

    Ok(info)
}

async fn run_relay_host_loop(
    app: tauri::AppHandle,
    relay: String,
    token: String,
    relay_cert_sha256: Option<String>,
) {
    let cert_sha256 = match relay_cert_sha256.as_deref() {
        Some(value) => match rterm_protocol::parse_sha256_fingerprint(value) {
            Ok(value) => Some(value),
            Err(err) => {
                let _ = app.emit(
                    "service-status",
                    format!("relay certificate pin is invalid: {err:#}"),
                );
                return;
            }
        },
        None => None,
    };
    let relay_target = match rterm_protocol::resolve_target(&relay) {
        Ok(value) => value,
        Err(err) => {
            let _ = app.emit("service-status", format!("relay resolve failed: {err:#}"));
            return;
        }
    };

    loop {
        let _ = app.emit(
            "service-status",
            format!("connecting to relay {}", relay_target.addr),
        );
        match relay_once(&relay_target, &token, cert_sha256).await {
            Ok(()) => {
                let _ = app.emit("service-status", "relay session ended; reconnecting");
            }
            Err(err) => {
                let _ = app.emit(
                    "service-status",
                    format!("relay session failed: {err:#}; reconnecting"),
                );
            }
        }
        tokio::time::sleep(config::RELAY_RECONNECT_DELAY).await;
    }
}

async fn relay_once(
    relay: &rterm_protocol::Target,
    token: &str,
    cert_sha256: Option<[u8; config::SHA256_LEN]>,
) -> Result<()> {
    let mut endpoint = Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
    endpoint.set_default_client_config(if let Some(expected_sha256) = cert_sha256 {
        rterm_protocol::pinned_client_config(expected_sha256)?
    } else {
        rterm_protocol::trusted_client_config()?
    });

    let connection = endpoint
        .connect(relay.addr, &relay.server_name)?
        .await
        .context("connect to relay")?;
    let (mut send, mut recv) = connection.open_bi().await.context("open relay stream")?;

    rterm_protocol::write_frame(&mut send, config::HELLO_HOST).await?;

    let challenge = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("relay closed before auth challenge")?;
    let nonce = rterm_protocol::parse_challenge_frame(&challenge)?;
    let auth = rterm_protocol::auth_response_frame(config::ROLE_HOST, token.as_bytes(), &nonce)?;
    rterm_protocol::write_frame(&mut send, &auth).await?;

    let response = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("relay closed before response")?;
    match response.as_ref() {
        config::RESPONSE_WAIT => {}
        config::RESPONSE_OK => {}
        other => anyhow::bail!("relay rejected host: {}", String::from_utf8_lossy(other)),
    }

    if response.as_ref() == config::RESPONSE_WAIT {
        let paired = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
            .await?
            .context("relay closed before pairing")?;
        if paired.as_ref() != config::RESPONSE_OK {
            anyhow::bail!("relay rejected host: {}", String::from_utf8_lossy(&paired));
        }
    }

    crate::pty::run_pty(send, recv, TerminalShell::default()).await?;
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    connection.close(config::CLOSE_NORMAL.into(), config::CLOSE_REASON_DONE);
    endpoint.wait_idle().await;
    Ok(())
}

fn parse_listen_port(value: &str) -> Result<u16> {
    value
        .parse::<SocketAddr>()
        .map(|addr| addr.port())
        .context("parse host listen address")
}

fn bind_endpoint(port: u16) -> Result<(Endpoint, String)> {
    let generated = rterm_protocol::self_signed_server_config(vec![
        config::DEFAULT_DIRECT_SERVER_NAME.to_string(),
    ])?;
    let requested = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    match Endpoint::server(generated.config, requested) {
        Ok(endpoint) => Ok((endpoint, generated.certificate_sha256_pin)),
        Err(_) => {
            let generated = rterm_protocol::self_signed_server_config(vec![
                config::DEFAULT_DIRECT_SERVER_NAME.to_string(),
            ])?;
            let endpoint = Endpoint::server(
                generated.config,
                SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            )?;
            Ok((endpoint, generated.certificate_sha256_pin))
        }
    }
}

async fn handle_connection(
    connecting: quinn::Incoming,
    settings: crate::HostSettings,
    token: String,
) -> Result<()> {
    let connection = connecting.await.context("accept QUIC connection")?;
    let (mut send, mut recv) = connection.accept_bi().await.context("accept stream")?;

    let hello = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT),
    )
    .await
    .context("client hello timed out")??
    .context("client closed before hello")?;
    if hello.as_ref() != config::HELLO_CLIENT {
        write_error_and_finish(&mut send, config::RESPONSE_ERR_BAD_HELLO).await?;
        return Ok(());
    }

    let nonce = rterm_protocol::new_auth_challenge()?;
    rterm_protocol::write_frame(&mut send, &rterm_protocol::challenge_frame(&nonce)).await?;

    let received = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::AUTH_FRAME_LIMIT),
    )
    .await
    .context("client auth timed out")??
    .context("client closed before auth")?;

    let auth_secret = auth_secret(&token, settings.password.as_deref());
    match rterm_protocol::verify_auth_response(received.as_ref(), auth_secret.as_bytes(), &nonce) {
        Ok(role) if role == config::ROLE_CLIENT => {}
        _ => {
            write_error_and_finish(&mut send, config::RESPONSE_ERR_AUTH_FAILED).await?;
            return Ok(());
        }
    }

    rterm_protocol::write_frame(&mut send, config::RESPONSE_OK).await?;
    crate::pty::run_pty(send, recv, settings.shell).await?;
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    Ok(())
}

async fn write_error_and_finish(send: &mut quinn::SendStream, frame: &[u8]) -> Result<()> {
    rterm_protocol::write_frame(send, frame).await?;
    let _ = send.finish();
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    Ok(())
}

struct UpnpMapping {
    gateway: igd::aio::Gateway,
    external_port: u16,
    external_ip: Ipv4Addr,
}

impl Drop for UpnpMapping {
    fn drop(&mut self) {
        let gateway = self.gateway.clone();
        let external_port = self.external_port;
        tokio::spawn(async move {
            let _ = gateway
                .remove_port(PortMappingProtocol::UDP, external_port)
                .await;
        });
    }
}

async fn add_upnp_mapping(
    listen: SocketAddr,
    external_port: u16,
    lease_seconds: u32,
) -> Result<UpnpMapping> {
    let local_ip = match listen.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => ip,
        IpAddr::V4(_) => match lan_ip() {
            Some(IpAddr::V4(ip)) => ip,
            _ => anyhow::bail!("could not determine local IPv4 for UPnP"),
        },
        IpAddr::V6(_) => anyhow::bail!("UPnP IGD only supports IPv4"),
    };
    let local_addr = SocketAddrV4::new(local_ip, listen.port());
    let gateway = search_gateway(SearchOptions::default())
        .await
        .context("find UPnP IGD gateway")?;
    gateway
        .add_port(
            PortMappingProtocol::UDP,
            external_port,
            local_addr,
            lease_seconds,
            config::UPNP_DESCRIPTION,
        )
        .await
        .with_context(|| format!("add UPnP UDP mapping {external_port} -> {local_addr}"))?;
    let external_ip = gateway
        .get_external_ip()
        .await
        .context("get UPnP external IP")?;
    Ok(UpnpMapping {
        gateway,
        external_port,
        external_ip,
    })
}

fn auth_secret(token: &str, password: Option<&str>) -> String {
    match password.filter(|value| !value.is_empty()) {
        Some(password) => format!("{token}\0{password}"),
        None => token.to_string(),
    }
}

fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).ok()?;
    socket
        .connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 80))
        .ok()?;
    Some(socket.local_addr().ok()?.ip())
}

pub fn compact_pairing_json(pairing: &PairingPayload) -> Result<String> {
    let mode = match pairing.mode.as_str() {
        "direct" => "d",
        "tracker" => "t",
        "relay" => "r",
        other => anyhow::bail!("unsupported pairing mode: {other}"),
    };
    let mut value = serde_json::json!({
        "v": pairing.v,
        "m": mode,
        "k": pairing.token,
        "c": pairing.cert_sha256,
    });
    let object = value
        .as_object_mut()
        .context("compact pairing must be a JSON object")?;
    if let Some(host) = pairing.host.as_ref().filter(|value| !value.is_empty()) {
        object.insert("h".to_string(), serde_json::Value::String(host.clone()));
    }
    if let Some(tracker) = pairing.tracker.as_ref().filter(|value| !value.is_empty()) {
        object.insert("r".to_string(), serde_json::Value::String(tracker.clone()));
    }
    if let Some(room) = pairing
        .tracker_room
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        object.insert("q".to_string(), serde_json::Value::String(room.clone()));
    }
    if let Some(relay) = pairing.relay.as_ref().filter(|value| !value.is_empty()) {
        object.insert("y".to_string(), serde_json::Value::String(relay.clone()));
    }
    if let Some(pin) = pairing
        .relay_cert_sha256
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        object.insert("p".to_string(), serde_json::Value::String(pin.clone()));
    }
    if pairing.requires_password.unwrap_or(false) {
        object.insert("a".to_string(), serde_json::Value::Bool(true));
    }
    Ok(serde_json::to_string(&value)?)
}

pub fn render_qr_svg(data: &str) -> Result<String> {
    let code = qrcode::QrCode::new(data.as_bytes())?;
    Ok(code
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(256, 256)
        .dark_color(qrcode::render::svg::Color("#0e0e0e"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}
