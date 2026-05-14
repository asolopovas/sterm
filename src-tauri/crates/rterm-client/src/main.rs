use anyhow::{Context, Result};
use clap::Parser;
use quinn::{Endpoint, RecvStream, SendStream};
use rterm_protocol::config;
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, conflicts_with = "relay", help = "Direct host address")]
    host: Option<String>,

    #[arg(long, conflicts_with = "host", help = "Public relay address")]
    relay: Option<String>,

    #[arg(
        long,
        conflicts_with_all = ["host", "relay"],
        help = "UDP BitTorrent tracker for peer discovery"
    )]
    tracker: Option<String>,

    #[arg(long, default_value = rterm_protocol::DEFAULT_TRACKER_ROOM, help = "Shared tracker room name")]
    tracker_room: String,

    #[arg(long, conflicts_with = "token_file")]
    token: Option<String>,

    #[arg(long, env = "RTERM_TOKEN_FILE")]
    token_file: Option<PathBuf>,

    #[arg(
        long,
        help = "SHA-256 fingerprint for a self-signed direct host or local relay certificate"
    )]
    cert_sha256: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    rterm_protocol::install_crypto_provider();

    let token = rterm_protocol::load_token(args.token, args.token_file)?;

    let relay_mode = args.relay.is_some();
    let tracker_mode = args.tracker.is_some();
    let pinned_cert = args
        .cert_sha256
        .as_deref()
        .map(rterm_protocol::parse_sha256_fingerprint)
        .transpose()?;

    if tracker_mode {
        let expected_sha256 = pinned_cert.context("tracker mode requires --cert-sha256")?;
        let tracker_arg = args
            .tracker
            .as_deref()
            .context("tracker mode requires --tracker")?;
        let tracker = rterm_protocol::resolve_tracker(tracker_arg)?;
        let mut peers = Vec::new();
        for attempt in 1..=config::TRACKER_CLIENT_DISCOVERY_ATTEMPTS {
            let announce = rterm_protocol::announce_to_tracker(
                &tracker,
                token.as_bytes(),
                &args.tracker_room,
                1,
            )
            .await?;
            peers = announce.peers;
            if !peers.is_empty() {
                break;
            }
            eprintln!(
                "tracker returned no peers on attempt {attempt}; retrying in {:?}",
                config::TRACKER_RETRY_DELAY
            );
            tokio::time::sleep(config::TRACKER_RETRY_DELAY).await;
        }
        anyhow::ensure!(!peers.is_empty(), "tracker returned no peers");
        eprintln!(
            "tracker returned {} peer candidate(s): {:?}",
            peers.len(),
            peers
        );
        let mut last_err = None;
        for peer in peers {
            match connect_and_stream(
                peer,
                config::DEFAULT_DIRECT_SERVER_NAME,
                rterm_protocol::pinned_client_config(expected_sha256)?,
                &token,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(err) => {
                    eprintln!("candidate {peer} failed: {err:#}");
                    last_err = Some(err);
                }
            }
        }
        if let Some(err) = last_err {
            return Err(err).context("all tracker candidates failed");
        }
        anyhow::bail!("tracker returned no usable peers");
    }

    let target_arg =
        args.host.as_deref().or(args.relay.as_deref()).context(
            "pass --host HOST:PORT, --relay RELAY:PORT, or --tracker udp://TRACKER:PORT",
        )?;
    let target = rterm_protocol::resolve_target(target_arg)?;
    let client_config = if let Some(expected_sha256) = pinned_cert {
        rterm_protocol::pinned_client_config(expected_sha256)?
    } else if relay_mode {
        rterm_protocol::trusted_client_config()?
    } else {
        anyhow::bail!("direct mode requires --cert-sha256")
    };
    let server_name = if relay_mode {
        target.server_name.as_str()
    } else {
        config::DEFAULT_DIRECT_SERVER_NAME
    };

    connect_and_stream(target.addr, server_name, client_config, &token).await
}

async fn connect_and_stream(
    addr: SocketAddr,
    server_name: &str,
    client_config: quinn::ClientConfig,
    token: &str,
) -> Result<()> {
    let mut endpoint = Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
    endpoint.set_default_client_config(client_config);

    let connection = endpoint
        .connect(addr, server_name)?
        .await
        .context("connect QUIC")?;
    eprintln!("QUIC connected to {addr}");

    let (mut send, mut recv) = connection.open_bi().await.context("open stream")?;
    eprintln!("opened bidirectional stream");

    rterm_protocol::write_frame(&mut send, config::HELLO_CLIENT).await?;

    let challenge = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT),
    )
    .await
    .context("auth challenge timed out")??
    .context("server closed before auth challenge")?;
    let nonce = rterm_protocol::parse_challenge_frame(&challenge)?;
    let auth_response =
        rterm_protocol::auth_response_frame(config::ROLE_CLIENT, token.as_bytes(), &nonce)?;
    rterm_protocol::write_frame(&mut send, &auth_response).await?;

    let auth = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("server closed before auth response")?;
    if auth.as_ref() != config::RESPONSE_OK {
        anyhow::bail!("connection rejected: {}", String::from_utf8_lossy(&auth));
    }

    eprintln!("connected. Press Ctrl-D or close the shell to exit.");
    stream_terminal(send, recv).await?;

    connection.close(config::CLOSE_NORMAL.into(), config::CLOSE_REASON_DONE);
    endpoint.wait_idle().await;
    Ok(())
}

async fn stream_terminal(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(config::STDIN_CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; config::IO_BUFFER_SIZE];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdin_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let to_host = tokio::spawn(async move {
        while let Some(data) = stdin_rx.recv().await {
            rterm_protocol::write_frame(&mut send, &data).await?;
        }
        let _ = send.finish();
        Ok::<_, anyhow::Error>(())
    });

    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let stdout_thread = std::thread::spawn(move || -> Result<()> {
        let mut stdout = std::io::stdout();
        for data in stdout_rx {
            stdout.write_all(&data)?;
            stdout.flush()?;
        }
        Ok(())
    });

    let mut from_host = tokio::spawn(async move {
        while let Some(data) = rterm_protocol::read_frame(&mut recv).await? {
            if stdout_tx.send(data.to_vec()).is_err() {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    tokio::select! {
        result = &mut from_host => result??,
        _ = tokio::signal::ctrl_c() => {
            from_host.abort();
            let _ = from_host.await;
        },
    }
    to_host.abort();
    let _ = to_host.await;
    stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stdout thread panicked"))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_reject_host_and_relay_together() {
        assert!(Args::try_parse_from([
            "rterm-client",
            "--host",
            "127.0.0.1:4433",
            "--relay",
            "relay.example:443",
        ])
        .is_err());
    }

    #[test]
    fn args_reject_tracker_with_direct_host() {
        assert!(Args::try_parse_from([
            "rterm-client",
            "--host",
            "127.0.0.1:4433",
            "--tracker",
            "127.0.0.1:6969",
        ])
        .is_err());
    }

    #[test]
    fn args_reject_token_and_token_file_together() {
        assert!(Args::try_parse_from([
            "rterm-client",
            "--host",
            "127.0.0.1:4433",
            "--token",
            "12345678901234567890123456789012",
            "--token-file",
            "token.txt",
        ])
        .is_err());
    }

    #[test]
    fn args_accept_tracker_mode() {
        assert!(Args::try_parse_from([
            "rterm-client",
            "--tracker",
            "127.0.0.1:6969",
            "--cert-sha256",
            &"01".repeat(32),
        ])
        .is_ok());
    }
}
