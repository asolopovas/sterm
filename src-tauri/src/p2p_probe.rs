use crate::TerminalEvent;
use anyhow::{bail, Context, Result};
use rand::{rngs::OsRng, RngCore};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};
use tauri::ipc::Channel;
use tokio::net::UdpSocket;

const DEFAULT_STUN_SERVER: &str = "stun.l.google.com:19302";
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_SUCCESS: u16 = 0x0101;
const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const STUN_ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const STUN_ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const PROBE_DURATION: Duration = Duration::from_secs(12);
const PROBE_TICK: Duration = Duration::from_millis(500);

pub async fn discover_public_ip_via_stun() -> Result<IpAddr> {
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))
        .await
        .context("bind UDP STUN socket")?;
    Ok(stun_public_addr(&socket).await?.ip())
}

pub async fn run(tracker: String, room: String, on_event: Channel<TerminalEvent>) -> Result<()> {
    let tracker = rterm_protocol::resolve_tracker(&tracker)?;
    let bind = match tracker.addr.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind)
        .await
        .context("bind UDP probe socket")?;
    let local_addr = socket
        .local_addr()
        .context("read UDP probe socket address")?;
    emit(&on_event, format!("probe UDP socket bound to {local_addr}"));

    let public_addr = match stun_public_addr(&socket).await {
        Ok(addr) => {
            emit(&on_event, format!("STUN public mapping: {addr}"));
            Some(addr)
        }
        Err(err) => {
            emit(&on_event, format!("STUN failed: {err:#}"));
            None
        }
    };
    let announced_port = public_addr.map_or(local_addr.port(), |addr| addr.port());
    emit(
        &on_event,
        format!(
            "announcing to tracker {} room '{room}' as UDP port {announced_port}",
            tracker.addr
        ),
    );

    emit(
        &on_event,
        "discovering peers and sending simultaneous UDP hole-punch probes".to_string(),
    );
    let token = probe_token(&room);
    let nonce = random_nonce();
    let deadline = Instant::now() + PROBE_DURATION;
    let mut buf = [0u8; 256];
    let mut peers = HashSet::new();
    let mut received = HashSet::new();
    let mut attempt = 0;

    while Instant::now() < deadline {
        attempt += 1;
        match rterm_protocol::announce_with_socket(
            &socket,
            tracker.addr,
            &token,
            &room,
            announced_port,
        )
        .await
        {
            Ok(announce) => {
                let before = peers.len();
                peers.extend(
                    announce
                        .peers
                        .into_iter()
                        .filter(|peer| Some(*peer) != public_addr && *peer != local_addr),
                );
                emit(
                    &on_event,
                    format!(
                        "tracker attempt {attempt} knows {} peer(s): {peers:?}",
                        peers.len()
                    ),
                );
                if peers.len() > before {
                    emit(
                        &on_event,
                        "new peer candidate found; punching now".to_string(),
                    );
                }
            }
            Err(err) => emit(
                &on_event,
                format!("tracker attempt {attempt} failed: {err:#}"),
            ),
        }

        let sleep = tokio::time::sleep(PROBE_TICK);
        tokio::pin!(sleep);
        loop {
            for peer in &peers {
                let message = format!("sterm-p2p-probe:{room}:{nonce}");
                let _ = socket.send_to(message.as_bytes(), peer).await;
            }
            tokio::select! {
                _ = &mut sleep => break,
                recv = socket.recv_from(&mut buf) => {
                    let (len, from) = recv.context("receive UDP probe")?;
                    if peers.contains(&from) {
                        let text = String::from_utf8_lossy(&buf[..len]);
                        if text.starts_with("sterm-p2p-probe:") && received.insert(from) {
                            emit(&on_event, format!("received UDP probe from {from}"));
                        }
                    }
                }
            }
        }
    }

    if peers.is_empty() {
        bail!("no probe peer found; run this test on both devices at the same time with the same room");
    }
    if received.is_empty() {
        bail!("no UDP probe received; this NAT combination is not hole-punchable without UPnP/port-forwarding or relay fallback");
    }

    emit(
        &on_event,
        format!("P2P UDP probe succeeded with {} peer(s)", received.len()),
    );
    Ok(())
}

async fn stun_public_addr(socket: &UdpSocket) -> Result<SocketAddr> {
    let server = tokio::net::lookup_host(DEFAULT_STUN_SERVER)
        .await
        .context("resolve STUN server")?
        .next()
        .context("STUN server resolved no addresses")?;
    let mut tx = [0u8; 12];
    OsRng.fill_bytes(&mut tx);
    let mut request = Vec::with_capacity(20);
    request.extend_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    request.extend_from_slice(&0u16.to_be_bytes());
    request.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    request.extend_from_slice(&tx);
    socket
        .send_to(&request, server)
        .await
        .context("send STUN binding request")?;

    let mut buf = [0u8; 1024];
    let (len, from) = tokio::time::timeout(Duration::from_secs(4), socket.recv_from(&mut buf))
        .await
        .context("STUN binding timed out")?
        .context("receive STUN binding response")?;
    if from.ip() != server.ip() || len < 20 {
        bail!("unexpected STUN response from {from}");
    }
    parse_stun_addr(&buf[..len], &tx).context("parse STUN public address")
}

fn parse_stun_addr(buf: &[u8], tx: &[u8; 12]) -> Result<SocketAddr> {
    let message_type = u16::from_be_bytes([buf[0], buf[1]]);
    if message_type != STUN_BINDING_SUCCESS {
        bail!("unexpected STUN message type 0x{message_type:04x}");
    }
    let message_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if cookie != STUN_MAGIC_COOKIE || &buf[8..20] != tx || buf.len() < 20 + message_len {
        bail!("invalid STUN response header");
    }

    let mut pos = 20;
    while pos + 4 <= 20 + message_len {
        let attr_type = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let attr_len = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
        pos += 4;
        if pos + attr_len > buf.len() {
            bail!("truncated STUN attribute");
        }
        if attr_type == STUN_ATTR_XOR_MAPPED_ADDRESS || attr_type == STUN_ATTR_MAPPED_ADDRESS {
            return parse_stun_mapped_addr(attr_type, &buf[pos..pos + attr_len]);
        }
        pos += (attr_len + 3) & !3;
    }
    bail!("STUN response did not include mapped address")
}

fn parse_stun_mapped_addr(attr_type: u16, value: &[u8]) -> Result<SocketAddr> {
    if value.len() < 8 || value[1] != 0x01 {
        bail!("only IPv4 STUN mapped addresses are supported");
    }
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    let mut octets = [value[4], value[5], value[6], value[7]];
    if attr_type == STUN_ATTR_XOR_MAPPED_ADDRESS {
        port ^= (STUN_MAGIC_COOKIE >> 16) as u16;
        let cookie = STUN_MAGIC_COOKIE.to_be_bytes();
        for (octet, mask) in octets.iter_mut().zip(cookie) {
            *octet ^= mask;
        }
    }
    Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port))
}

fn probe_token(room: &str) -> Vec<u8> {
    format!("sterm-p2p-probe:{room}").into_bytes()
}

fn random_nonce() -> String {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn emit(on_event: &Channel<TerminalEvent>, message: String) {
    let _ = on_event.send(TerminalEvent::Status(message));
}
