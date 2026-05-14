use anyhow::{anyhow, bail, Context, Result};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    time::Duration,
};
use tokio::{net::UdpSocket, time};

use crate::config;

type HmacSha256 = Hmac<Sha256>;

const UDP_TRACKER_PROTOCOL_ID: i64 = 0x41727101980;
const ACTION_CONNECT: i32 = 0;
const ACTION_ANNOUNCE: i32 = 1;
const EVENT_STARTED: i32 = 2;
const DEFAULT_NUMWANT: i32 = 16;
const DEFAULT_BYTES_LEFT: i64 = 1;
const CONNECT_REQUEST_LEN: usize = 16;
const CONNECT_RESPONSE_LEN: usize = 16;
const ANNOUNCE_REQUEST_LEN: usize = 98;
const ANNOUNCE_RESPONSE_FIXED_LEN: usize = 20;
const IPV4_PEER_LEN: usize = 6;
const IPV6_PEER_LEN: usize = 18;

pub const DEFAULT_TRACKER_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_TRACKER_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(120);
pub const DEFAULT_TRACKER_ROOM: &str = "default";

#[derive(Debug, Clone)]
pub struct TrackerTarget {
    pub addr: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct TrackerAnnounce {
    pub peers: Vec<SocketAddr>,
    pub interval: Duration,
}

pub async fn announce_to_tracker(
    tracker: &TrackerTarget,
    token: &[u8],
    room: &str,
    listen_port: u16,
) -> Result<TrackerAnnounce> {
    let bind = match tracker.addr.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind)
        .await
        .context("bind UDP tracker socket")?;
    announce_with_socket(&socket, tracker.addr, token, room, listen_port).await
}

pub async fn announce_with_socket(
    socket: &UdpSocket,
    tracker: SocketAddr,
    token: &[u8],
    room: &str,
    listen_port: u16,
) -> Result<TrackerAnnounce> {
    let transaction_id = random_i32()?;
    let mut connect = Vec::with_capacity(CONNECT_REQUEST_LEN);
    push_i64(&mut connect, UDP_TRACKER_PROTOCOL_ID);
    push_i32(&mut connect, ACTION_CONNECT);
    push_i32(&mut connect, transaction_id);
    socket
        .send_to(&connect, tracker)
        .await
        .context("send UDP tracker connect")?;

    let mut buf = [0u8; 2048];
    let (len, src) = time::timeout(DEFAULT_TRACKER_TIMEOUT, socket.recv_from(&mut buf))
        .await
        .context("UDP tracker connect timed out")?
        .context("receive UDP tracker connect")?;
    ensure_tracker_source(src, tracker)?;
    let connection_id = parse_connect_response(&buf[..len], transaction_id)?;

    let transaction_id = random_i32()?;
    let info_hash = tracker_info_hash(token, room)?;
    let peer_id = tracker_peer_id(token, room)?;
    let mut announce = Vec::with_capacity(ANNOUNCE_REQUEST_LEN);
    push_i64(&mut announce, connection_id);
    push_i32(&mut announce, ACTION_ANNOUNCE);
    push_i32(&mut announce, transaction_id);
    announce.extend_from_slice(&info_hash);
    announce.extend_from_slice(&peer_id);
    push_i64(&mut announce, 0);
    push_i64(&mut announce, DEFAULT_BYTES_LEFT);
    push_i64(&mut announce, 0);
    push_i32(&mut announce, EVENT_STARTED);
    push_u32(&mut announce, 0);
    push_i32(&mut announce, random_i32()?);
    push_i32(&mut announce, DEFAULT_NUMWANT);
    push_u16(&mut announce, listen_port);
    socket
        .send_to(&announce, tracker)
        .await
        .context("send UDP tracker announce")?;

    let (len, src) = time::timeout(DEFAULT_TRACKER_TIMEOUT, socket.recv_from(&mut buf))
        .await
        .context("UDP tracker announce timed out")?
        .context("receive UDP tracker announce")?;
    ensure_tracker_source(src, tracker)?;
    parse_announce_response(&buf[..len], transaction_id, tracker.is_ipv4())
}

pub fn resolve_tracker(value: &str) -> Result<TrackerTarget> {
    let without_scheme = value.strip_prefix("udp://").unwrap_or(value);
    anyhow::ensure!(
        !without_scheme.contains('/'),
        "UDP tracker must be udp://HOST:PORT or HOST:PORT"
    );
    let addr = without_scheme
        .to_socket_addrs()
        .with_context(|| format!("resolve UDP tracker {value}"))?
        .next()
        .with_context(|| format!("UDP tracker {value} resolved no addresses"))?;
    Ok(TrackerTarget { addr })
}

pub fn tracker_info_hash(token: &[u8], room: &str) -> Result<[u8; 20]> {
    let digest = tracker_hmac(token, b"info-hash", room)?;
    let mut out = [0u8; 20];
    out.copy_from_slice(&digest[..20]);
    Ok(out)
}

pub fn tracker_peer_id(token: &[u8], room: &str) -> Result<[u8; 20]> {
    let digest = tracker_hmac(token, b"peer-id", room)?;
    let mut out = [0u8; 20];
    out[..8].copy_from_slice(b"-RT0001-");
    out[8..].copy_from_slice(&digest[..12]);
    Ok(out)
}

fn tracker_hmac(token: &[u8], label: &[u8], room: &str) -> Result<[u8; 32]> {
    let mut mac = HmacSha256::new_from_slice(token).map_err(|_| anyhow!("invalid token"))?;
    mac.update(b"rterm-poc-tracker-v1");
    mac.update(config::WIRE_SEPARATOR);
    mac.update(label);
    mac.update(config::WIRE_SEPARATOR);
    mac.update(room.as_bytes());
    Ok(mac.finalize().into_bytes().into())
}

fn parse_connect_response(bytes: &[u8], transaction_id: i32) -> Result<i64> {
    anyhow::ensure!(
        bytes.len() >= CONNECT_RESPONSE_LEN,
        "short UDP tracker connect response"
    );
    let action = read_i32(bytes, 0)?;
    let got_transaction_id = read_i32(bytes, 4)?;
    if action == 3 {
        bail!(
            "UDP tracker error: {}",
            String::from_utf8_lossy(&bytes[8..])
        );
    }
    anyhow::ensure!(
        action == ACTION_CONNECT,
        "bad UDP tracker connect action {action}"
    );
    anyhow::ensure!(
        got_transaction_id == transaction_id,
        "UDP tracker connect transaction id mismatch"
    );
    read_i64(bytes, 8)
}

fn parse_announce_response(
    bytes: &[u8],
    transaction_id: i32,
    ipv4: bool,
) -> Result<TrackerAnnounce> {
    anyhow::ensure!(
        bytes.len() >= ANNOUNCE_RESPONSE_FIXED_LEN,
        "short UDP tracker announce response"
    );
    let action = read_i32(bytes, 0)?;
    let got_transaction_id = read_i32(bytes, 4)?;
    if action == 3 {
        bail!(
            "UDP tracker error: {}",
            String::from_utf8_lossy(&bytes[8..])
        );
    }
    anyhow::ensure!(
        action == ACTION_ANNOUNCE,
        "bad UDP tracker announce action {action}"
    );
    anyhow::ensure!(
        got_transaction_id == transaction_id,
        "UDP tracker announce transaction id mismatch"
    );
    let interval_secs = read_i32(bytes, 8)?.max(1) as u64;
    let peer_len = if ipv4 { IPV4_PEER_LEN } else { IPV6_PEER_LEN };
    let peer_bytes = &bytes[ANNOUNCE_RESPONSE_FIXED_LEN..];
    anyhow::ensure!(
        peer_bytes.len().is_multiple_of(peer_len),
        "invalid compact peer list length"
    );
    let peers = if ipv4 {
        parse_ipv4_peers(peer_bytes)
    } else {
        parse_ipv6_peers(peer_bytes)
    };
    Ok(TrackerAnnounce {
        peers,
        interval: Duration::from_secs(interval_secs),
    })
}

fn parse_ipv4_peers(bytes: &[u8]) -> Vec<SocketAddr> {
    bytes
        .chunks_exact(IPV4_PEER_LEN)
        .filter_map(|chunk| {
            let port = u16::from_be_bytes([chunk[4], chunk[5]]);
            (port != 0).then(|| {
                SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3])),
                    port,
                )
            })
        })
        .collect()
}

fn parse_ipv6_peers(bytes: &[u8]) -> Vec<SocketAddr> {
    bytes
        .chunks_exact(IPV6_PEER_LEN)
        .filter_map(|chunk| {
            let port = u16::from_be_bytes([chunk[16], chunk[17]]);
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&chunk[..16]);
            (port != 0).then(|| SocketAddr::new(IpAddr::V6(octets.into()), port))
        })
        .collect()
}

fn ensure_tracker_source(src: SocketAddr, tracker: SocketAddr) -> Result<()> {
    anyhow::ensure!(
        src == tracker,
        "ignoring UDP tracker packet from unexpected source {src}"
    );
    Ok(())
}

fn random_i32() -> Result<i32> {
    let mut bytes = [0u8; 4];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| anyhow!("secure randomness unavailable"))?;
    Ok(i32::from_ne_bytes(bytes))
}

fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .context("read network i64")?
        .try_into()
        .context("copy network i64")?;
    Ok(i64::from_be_bytes(raw))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .context("read network i32")?
        .try_into()
        .context("copy network i32")?;
    Ok(i32::from_be_bytes(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_identity_is_stable_and_room_scoped() {
        let a = tracker_info_hash(b"token", "room-a").unwrap();
        let b = tracker_info_hash(b"token", "room-a").unwrap();
        let c = tracker_info_hash(b"token", "room-b").unwrap();

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(
            &tracker_peer_id(b"token", "room-a").unwrap()[..8],
            b"-RT0001-"
        );
    }

    #[test]
    fn parses_ipv4_announce_response() {
        let mut response = announce_response_header(7, 60);
        response.extend_from_slice(&[127, 0, 0, 1, 0x11, 0x5c]);

        let parsed = parse_announce_response(&response, 7, true).unwrap();

        assert_eq!(parsed.interval, Duration::from_secs(60));
        assert_eq!(parsed.peers, vec!["127.0.0.1:4444".parse().unwrap()]);
    }

    #[test]
    fn parses_ipv6_announce_response() {
        let mut response = announce_response_header(7, 60);
        response.extend_from_slice(&[0; 15]);
        response.push(1);
        response.extend_from_slice(&0x22b8u16.to_be_bytes());

        let parsed = parse_announce_response(&response, 7, false).unwrap();

        assert_eq!(parsed.peers, vec!["[::1]:8888".parse().unwrap()]);
    }

    #[test]
    fn announce_response_filters_zero_port_peers() {
        let mut response = announce_response_header(7, 60);
        response.extend_from_slice(&[127, 0, 0, 1, 0, 0]);

        let parsed = parse_announce_response(&response, 7, true).unwrap();

        assert!(parsed.peers.is_empty());
    }

    #[test]
    fn announce_response_rejects_invalid_peer_list_length() {
        let mut response = announce_response_header(7, 60);
        response.extend_from_slice(&[127, 0, 0, 1, 0x11]);

        assert!(parse_announce_response(&response, 7, true).is_err());
    }

    #[test]
    fn announce_response_rejects_tracker_error_action() {
        let mut response = Vec::new();
        push_i32(&mut response, 3);
        push_i32(&mut response, 7);
        response.extend_from_slice(b"failure");

        assert!(parse_announce_response(&response, 7, true).is_err());
    }

    #[test]
    fn rejects_wrong_transaction_id() {
        let mut response = Vec::new();
        push_i32(&mut response, ACTION_CONNECT);
        push_i32(&mut response, 1);
        push_i64(&mut response, 9);

        assert!(parse_connect_response(&response, 2).is_err());
    }

    #[test]
    fn connect_response_rejects_short_packet() {
        assert!(parse_connect_response(&[0; CONNECT_RESPONSE_LEN - 1], 2).is_err());
    }

    fn announce_response_header(transaction_id: i32, interval_secs: i32) -> Vec<u8> {
        let mut response = Vec::new();
        push_i32(&mut response, ACTION_ANNOUNCE);
        push_i32(&mut response, transaction_id);
        push_i32(&mut response, interval_secs);
        push_i32(&mut response, 1);
        push_i32(&mut response, 1);
        response
    }

    #[tokio::test]
    async fn announces_against_fake_udp_tracker() {
        crate::install_crypto_provider();
        let tracker = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let tracker_addr = tracker.local_addr().unwrap();
        let expected_info_hash = tracker_info_hash(b"test-token", "test-room").unwrap();

        let server = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let (len, client) = tracker.recv_from(&mut buf).await.unwrap();
            assert_eq!(len, CONNECT_REQUEST_LEN);
            assert_eq!(read_i64(&buf[..len], 0).unwrap(), UDP_TRACKER_PROTOCOL_ID);
            assert_eq!(read_i32(&buf[..len], 8).unwrap(), ACTION_CONNECT);
            let connect_tx = read_i32(&buf[..len], 12).unwrap();

            let mut connect_response = Vec::new();
            push_i32(&mut connect_response, ACTION_CONNECT);
            push_i32(&mut connect_response, connect_tx);
            push_i64(&mut connect_response, 12345);
            tracker.send_to(&connect_response, client).await.unwrap();

            let (len, client) = tracker.recv_from(&mut buf).await.unwrap();
            assert_eq!(len, ANNOUNCE_REQUEST_LEN);
            assert_eq!(read_i64(&buf[..len], 0).unwrap(), 12345);
            assert_eq!(read_i32(&buf[..len], 8).unwrap(), ACTION_ANNOUNCE);
            let announce_tx = read_i32(&buf[..len], 12).unwrap();
            assert_eq!(&buf[16..36], &expected_info_hash);
            assert_eq!(u16::from_be_bytes([buf[96], buf[97]]), 4444);

            let mut announce_response = Vec::new();
            push_i32(&mut announce_response, ACTION_ANNOUNCE);
            push_i32(&mut announce_response, announce_tx);
            push_i32(&mut announce_response, 30);
            push_i32(&mut announce_response, 1);
            push_i32(&mut announce_response, 1);
            announce_response.extend_from_slice(&[127, 0, 0, 1, 0x22, 0xb8]);
            tracker.send_to(&announce_response, client).await.unwrap();
        });

        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let announce =
            announce_with_socket(&client, tracker_addr, b"test-token", "test-room", 4444)
                .await
                .unwrap();

        server.await.unwrap();
        assert_eq!(announce.interval, Duration::from_secs(30));
        assert_eq!(announce.peers, vec!["127.0.0.1:8888".parse().unwrap()]);
    }
}
