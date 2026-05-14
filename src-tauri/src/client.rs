use crate::{AppState, PairingPayload, TerminalEvent};
use anyhow::{Context, Result};
use quinn::{Endpoint, RecvStream, SendStream};
use rterm_protocol::config;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tauri::ipc::Channel;
use tokio::sync::mpsc;

pub fn spawn_client_session(
    state: AppState,
    session_id: String,
    pairing: PairingPayload,
    mut input_rx: mpsc::Receiver<Vec<u8>>,
    on_event: Channel<TerminalEvent>,
) {
    tokio::spawn(async move {
        let result = connect_pairing(pairing, &mut input_rx, &on_event).await;
        if let Err(err) = result {
            let _ = on_event.send(TerminalEvent::Error(err.to_string()));
        }
        state.remove_session(&session_id);
        let _ = on_event.send(TerminalEvent::Closed);
    });
}

async fn connect_pairing(
    pairing: PairingPayload,
    input_rx: &mut mpsc::Receiver<Vec<u8>>,
    on_event: &Channel<TerminalEvent>,
) -> Result<()> {
    rterm_protocol::install_crypto_provider();
    match pairing.mode.as_str() {
        "direct" => connect_direct_pairing(pairing, input_rx, on_event).await,
        "tracker" => connect_tracker_pairing(pairing, input_rx, on_event).await,
        "relay" => connect_relay_pairing(pairing, input_rx, on_event).await,
        other => anyhow::bail!("unsupported pairing mode: {other}"),
    }
}

async fn connect_direct_pairing(
    pairing: PairingPayload,
    input_rx: &mut mpsc::Receiver<Vec<u8>>,
    on_event: &Channel<TerminalEvent>,
) -> Result<()> {
    let host = pairing.host.context("direct pairing is missing host")?;
    let addr: SocketAddr = host.parse().context("invalid host address in QR")?;
    let expected_sha256 = rterm_protocol::parse_sha256_fingerprint(&pairing.cert_sha256)?;
    connect_to_addr(
        addr,
        config::DEFAULT_DIRECT_SERVER_NAME,
        rterm_protocol::pinned_client_config(expected_sha256)?,
        &pairing.token,
        pairing.password.as_deref(),
        input_rx,
        on_event,
    )
    .await
}

async fn connect_tracker_pairing(
    pairing: PairingPayload,
    input_rx: &mut mpsc::Receiver<Vec<u8>>,
    on_event: &Channel<TerminalEvent>,
) -> Result<()> {
    let tracker_value = pairing
        .tracker
        .clone()
        .context("tracker pairing is missing tracker")?;
    let room = pairing
        .tracker_room
        .clone()
        .unwrap_or_else(|| rterm_protocol::DEFAULT_TRACKER_ROOM.to_string());
    let tracker = rterm_protocol::resolve_tracker(&tracker_value)?;
    let expected_sha256 = rterm_protocol::parse_sha256_fingerprint(&pairing.cert_sha256)?;
    let client_config = rterm_protocol::pinned_client_config(expected_sha256)?;

    if let Some(host) = pairing.host.as_deref() {
        let _ = on_event.send(TerminalEvent::Status(format!(
            "trying paired host {host} before tracker discovery"
        )));
        match host.parse::<SocketAddr>() {
            Ok(addr) => {
                match connect_to_addr(
                    addr,
                    config::DEFAULT_DIRECT_SERVER_NAME,
                    client_config.clone(),
                    &pairing.token,
                    pairing.password.as_deref(),
                    input_rx,
                    on_event,
                )
                .await
                {
                    Ok(()) => return Ok(()),
                    Err(err) => {
                        let _ = on_event.send(TerminalEvent::Status(format!(
                            "paired host failed: {err}; falling back to tracker"
                        )));
                    }
                }
            }
            Err(err) => {
                let _ = on_event.send(TerminalEvent::Status(format!(
                    "paired host address is invalid: {err}; falling back to tracker"
                )));
            }
        }
    }

    let mut last_discovery_err = None;
    for attempt in 1..=config::TRACKER_CLIENT_DISCOVERY_ATTEMPTS {
        let _ = on_event.send(TerminalEvent::Status(format!(
            "asking tracker for peers ({attempt})"
        )));
        let announce =
            match rterm_protocol::announce_to_tracker(&tracker, pairing.token.as_bytes(), &room, 1)
                .await
            {
                Ok(announce) => announce,
                Err(err) => {
                    let _ = on_event.send(TerminalEvent::Status(format!(
                        "tracker announce failed: {err}; retrying"
                    )));
                    last_discovery_err = Some(err);
                    tokio::time::sleep(config::TRACKER_RETRY_DELAY).await;
                    continue;
                }
            };
        if announce.peers.is_empty() {
            tokio::time::sleep(config::TRACKER_RETRY_DELAY).await;
            continue;
        }

        let _ = on_event.send(TerminalEvent::Status(format!(
            "tracker returned {} peer(s)",
            announce.peers.len()
        )));
        let mut last_err = None;
        for peer in announce.peers {
            match connect_to_addr(
                peer,
                config::DEFAULT_DIRECT_SERVER_NAME,
                client_config.clone(),
                &pairing.token,
                pairing.password.as_deref(),
                input_rx,
                on_event,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(err) => {
                    let _ =
                        on_event.send(TerminalEvent::Status(format!("peer {peer} failed: {err}")));
                    last_err = Some(err);
                }
            }
        }
        if let Some(err) = last_err {
            return Err(err).context("all tracker peers failed");
        }
    }

    if let Some(err) = last_discovery_err {
        return Err(err).context("tracker discovery failed");
    }
    anyhow::bail!("tracker returned no reachable peers")
}

async fn connect_relay_pairing(
    pairing: PairingPayload,
    input_rx: &mut mpsc::Receiver<Vec<u8>>,
    on_event: &Channel<TerminalEvent>,
) -> Result<()> {
    let relay_value = pairing.relay.context("relay pairing is missing relay")?;
    let target = rterm_protocol::resolve_target(&relay_value)?;
    let client_config = if let Some(pin) = pairing.relay_cert_sha256.as_deref() {
        rterm_protocol::pinned_client_config(rterm_protocol::parse_sha256_fingerprint(pin)?)?
    } else {
        rterm_protocol::trusted_client_config()?
    };
    connect_to_addr(
        target.addr,
        &target.server_name,
        client_config,
        &pairing.token,
        pairing.password.as_deref(),
        input_rx,
        on_event,
    )
    .await
}

async fn connect_to_addr(
    addr: SocketAddr,
    server_name: &str,
    client_config: quinn::ClientConfig,
    token: &str,
    password: Option<&str>,
    input_rx: &mut mpsc::Receiver<Vec<u8>>,
    on_event: &Channel<TerminalEvent>,
) -> Result<()> {
    let _ = on_event.send(TerminalEvent::Status(format!("connecting to {addr}")));

    let mut endpoint = Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
    endpoint.set_default_client_config(client_config);

    let connection = endpoint
        .connect(addr, server_name)?
        .await
        .context("connect QUIC")?;

    let (mut send, mut recv) = connection.open_bi().await.context("open stream")?;
    rterm_protocol::write_frame(&mut send, config::HELLO_CLIENT).await?;

    let challenge = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT),
    )
    .await
    .context("auth challenge timed out")??
    .context("server closed before auth challenge")?;
    let nonce = rterm_protocol::parse_challenge_frame(&challenge)?;
    let auth_secret = match password.filter(|value| !value.is_empty()) {
        Some(password) => format!("{token}\0{password}"),
        None => token.to_string(),
    };
    let auth_response =
        rterm_protocol::auth_response_frame(config::ROLE_CLIENT, auth_secret.as_bytes(), &nonce)?;
    rterm_protocol::write_frame(&mut send, &auth_response).await?;

    let auth = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("server closed before auth response")?;
    if auth.as_ref() != config::RESPONSE_OK {
        anyhow::bail!("connection rejected: {}", String::from_utf8_lossy(&auth));
    }

    let _ = on_event.send(TerminalEvent::Status("connected".to_string()));
    stream_terminal(&mut send, &mut recv, input_rx, on_event).await?;

    connection.close(config::CLOSE_NORMAL.into(), config::CLOSE_REASON_DONE);
    endpoint.wait_idle().await;
    Ok(())
}

async fn stream_terminal(
    send: &mut SendStream,
    recv: &mut RecvStream,
    input_rx: &mut mpsc::Receiver<Vec<u8>>,
    on_event: &Channel<TerminalEvent>,
) -> Result<()> {
    loop {
        tokio::select! {
            input = input_rx.recv() => {
                match input {
                    Some(data) => rterm_protocol::write_frame(send, &data).await?,
                    None => {
                        let _ = send.finish();
                        return Ok(());
                    }
                }
            }
            data = rterm_protocol::read_frame(recv) => {
                match data? {
                    Some(bytes) => {
                        let _ = on_event.send(TerminalEvent::Output(
                            String::from_utf8_lossy(&bytes).into_owned(),
                        ));
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}
