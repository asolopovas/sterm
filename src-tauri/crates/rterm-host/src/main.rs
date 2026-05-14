mod args;
mod pty;
mod upnp;

use anyhow::{Context, Result};
use args::Args;
use clap::Parser;
use pty::run_pty;
use quinn::Endpoint;
use rterm_protocol::config;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::sync::Semaphore;
use upnp::add_upnp_mapping;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    rterm_protocol::install_crypto_provider();

    let token = rterm_protocol::load_token(args.token, args.token_file)?;
    let relay_cert_sha256 = args
        .cert_sha256
        .as_deref()
        .map(rterm_protocol::parse_sha256_fingerprint)
        .transpose()?;

    if let Some(relay) = args.relay.as_deref() {
        run_relay_host(relay, &token, relay_cert_sha256).await
    } else {
        let listen = args
            .listen
            .context("pass either --listen ADDR:PORT or --relay RELAY:PORT")?;
        run_direct_host(
            listen,
            token,
            args.upnp,
            args.upnp_external_port,
            args.upnp_lease_seconds,
            args.tracker,
            args.tracker_room,
        )
        .await
    }
}

async fn run_relay_host(
    relay: &str,
    token: &str,
    cert_sha256: Option<[u8; config::SHA256_LEN]>,
) -> Result<()> {
    let relay = rterm_protocol::resolve_target(relay)?;

    loop {
        eprintln!("connecting to relay at {}...", relay.addr);
        match relay_once(&relay, token, cert_sha256).await {
            Ok(()) => eprintln!("relay session ended; reconnecting"),
            Err(err) => eprintln!("relay session failed: {err:#}; reconnecting"),
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
        config::RESPONSE_WAIT => eprintln!("authenticated with relay; waiting for client..."),
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

    eprintln!("client paired through relay; starting PTY");
    run_pty(send, recv).await?;
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    connection.close(config::CLOSE_NORMAL.into(), config::CLOSE_REASON_DONE);
    endpoint.wait_idle().await;
    Ok(())
}

async fn run_direct_host(
    listen: SocketAddr,
    token: String,
    upnp: bool,
    upnp_external_port: Option<u16>,
    upnp_lease_seconds: u32,
    tracker: Option<String>,
    tracker_room: String,
) -> Result<()> {
    let generated = rterm_protocol::self_signed_server_config(vec![
        config::DEFAULT_DIRECT_SERVER_NAME.to_string(),
    ])?;
    rterm_protocol::write_public_pin(
        config::DIRECT_CERT_PIN_LABEL,
        &generated.certificate_sha256_pin,
    )?;
    let endpoint = Endpoint::server(generated.config, listen)?;
    eprintln!("rterm-host listening on udp://{}", listen);

    let upnp_mapping = if upnp {
        let mapping = add_upnp_mapping(
            listen,
            upnp_external_port.unwrap_or(listen.port()),
            upnp_lease_seconds,
        )
        .await?;
        eprintln!(
            "from phone on cellular run: rterm-client --host {}:{} --token <token>",
            mapping.external_ip(),
            mapping.external_port()
        );
        Some(mapping)
    } else {
        None
    };

    let tracker_task = if let Some(tracker) = tracker {
        let tracker = rterm_protocol::resolve_tracker(&tracker)?;
        let token = token.clone();
        let announce_port = upnp_mapping
            .as_ref()
            .map(|mapping| mapping.external_port())
            .unwrap_or(listen.port());
        Some(tokio::spawn(async move {
            loop {
                match rterm_protocol::announce_to_tracker(
                    &tracker,
                    token.as_bytes(),
                    &tracker_room,
                    announce_port,
                )
                .await
                {
                    Ok(announce) => {
                        eprintln!(
                            "announced to tracker {}; {} peer(s), next in {:?}",
                            tracker.addr,
                            announce.peers.len(),
                            announce.interval
                        );
                        tokio::time::sleep(announce.interval).await;
                    }
                    Err(err) => {
                        eprintln!("tracker announce failed: {err:#}");
                        tokio::time::sleep(config::TRACKER_RETRY_DELAY).await;
                    }
                }
            }
        }))
    } else {
        None
    };

    let connection_limit = Arc::new(Semaphore::new(config::DIRECT_HOST_CONNECTION_LIMIT));

    loop {
        tokio::select! {
            Some(connecting) = endpoint.accept() => {
                let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
                    eprintln!("connection limit reached; dropping incoming connection");
                    continue;
                };
                let token = token.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(err) = handle_connection(connecting, token).await {
                        eprintln!("connection ended: {err:#}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutting down");
                break;
            }
        }
    }

    if let Some(task) = tracker_task {
        task.abort();
    }

    if let Some(mapping) = upnp_mapping {
        mapping.remove().await;
    }

    Ok(())
}

async fn handle_connection(connecting: quinn::Incoming, token: String) -> Result<()> {
    let connection = connecting.await.context("accept QUIC connection")?;
    let peer = connection.remote_address();
    eprintln!("accepted QUIC connection from {peer}");

    let (mut send, mut recv) = connection.accept_bi().await.context("accept stream")?;
    eprintln!("accepted bidirectional stream from {peer}");

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
    eprintln!("sent auth challenge to {peer}");

    let received = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::AUTH_FRAME_LIMIT),
    )
    .await
    .context("client auth timed out")??
    .context("client closed before auth")?;

    match rterm_protocol::verify_auth_response(received.as_ref(), token.as_bytes(), &nonce) {
        Ok(role) if role == config::ROLE_CLIENT => {}
        _ => {
            write_error_and_finish(&mut send, config::RESPONSE_ERR_AUTH_FAILED).await?;
            return Ok(());
        }
    }

    rterm_protocol::write_frame(&mut send, config::RESPONSE_OK).await?;
    run_pty(send, recv).await?;
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    Ok(())
}

async fn write_error_and_finish(send: &mut quinn::SendStream, frame: &[u8]) -> Result<()> {
    rterm_protocol::write_frame(send, frame).await?;
    let _ = send.finish();
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    Ok(())
}
