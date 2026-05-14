mod identity;

use anyhow::{Context, Result};
use clap::Parser;
use quinn::{Endpoint, RecvStream, SendStream};
use rterm_protocol::config;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, Semaphore};

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    listen: SocketAddr,

    #[arg(long, conflicts_with = "token_file")]
    token: Option<String>,

    #[arg(long, env = "RTERM_TOKEN_FILE")]
    token_file: Option<PathBuf>,

    #[arg(long, requires = "key", help = "PEM certificate chain")]
    cert: Option<PathBuf>,

    #[arg(long, requires = "cert", help = "PEM private key")]
    key: Option<PathBuf>,
}

struct WaitingHost {
    send: SendStream,
    recv: RecvStream,
}

type Waiting = Arc<Mutex<Option<WaitingHost>>>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    rterm_protocol::install_crypto_provider();

    let token = rterm_protocol::load_token(args.token, args.token_file)?;
    let endpoint = Endpoint::server(identity::server_config(args.cert, args.key)?, args.listen)?;
    let waiting: Waiting = Arc::new(Mutex::new(None));

    eprintln!("rterm-relay listening on udp://{}", args.listen);

    let connection_limit = Arc::new(Semaphore::new(config::RELAY_CONNECTION_LIMIT));

    loop {
        tokio::select! {
            Some(connecting) = endpoint.accept() => {
                let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
                    eprintln!("relay connection limit reached; dropping incoming connection");
                    continue;
                };
                let token = token.clone();
                let waiting = waiting.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(err) = handle_connection(connecting, token, waiting).await {
                        eprintln!("relay connection ended: {err:#}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutting down relay");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    connecting: quinn::Incoming,
    token: String,
    waiting: Waiting,
) -> Result<()> {
    let connection = connecting.await.context("accept QUIC connection")?;
    let peer = connection.remote_address();
    let (mut send, mut recv) = connection.accept_bi().await.context("accept stream")?;

    let hello = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT),
    )
    .await
    .context("relay hello timed out")??
    .context("peer closed before relay hello")?;
    let expected_role = match hello.as_ref() {
        config::HELLO_HOST => config::ROLE_HOST,
        config::HELLO_CLIENT => config::ROLE_CLIENT,
        _ => {
            write_error_and_finish(&mut send, config::RESPONSE_ERR_BAD_HELLO).await?;
            return Ok(());
        }
    };

    let nonce = rterm_protocol::new_auth_challenge()?;
    rterm_protocol::write_frame(&mut send, &rterm_protocol::challenge_frame(&nonce)).await?;

    let auth = tokio::time::timeout(
        config::AUTH_TIMEOUT,
        rterm_protocol::read_frame_limited(&mut recv, config::AUTH_FRAME_LIMIT),
    )
    .await
    .context("relay auth timed out")??
    .context("peer closed before relay auth")?;

    let role = match rterm_protocol::verify_auth_response(auth.as_ref(), token.as_bytes(), &nonce) {
        Ok(role) => role,
        Err(_) => {
            write_error_and_finish(&mut send, config::RESPONSE_ERR_AUTH_FAILED).await?;
            return Ok(());
        }
    };

    if role.as_slice() != expected_role {
        write_error_and_finish(&mut send, config::RESPONSE_ERR_ROLE_MISMATCH).await?;
        return Ok(());
    }

    if role.as_slice() == config::ROLE_HOST {
        eprintln!("host registered from {peer}");
        rterm_protocol::write_frame(&mut send, config::RESPONSE_WAIT).await?;
        let old = {
            let mut slot = waiting.lock().await;
            slot.replace(WaitingHost { send, recv })
        };
        if let Some(mut old) = old {
            let _ = rterm_protocol::write_frame(&mut old.send, config::RESPONSE_ERR_REPLACED_HOST)
                .await;
            let _ = old.send.finish();
        }
    } else {
        eprintln!("client connected from {peer}");
        let host = waiting.lock().await.take();
        let Some(host) = host else {
            write_error_and_finish(&mut send, config::RESPONSE_ERR_NO_HOST_WAITING).await?;
            return Ok(());
        };

        pair(host, WaitingHost { send, recv }).await?;
        tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    }

    Ok(())
}

async fn write_error_and_finish(send: &mut SendStream, frame: &[u8]) -> Result<()> {
    rterm_protocol::write_frame(send, frame).await?;
    let _ = send.finish();
    tokio::time::sleep(config::ERROR_RESPONSE_GRACE).await;
    Ok(())
}

async fn pair(mut host: WaitingHost, mut client: WaitingHost) -> Result<()> {
    rterm_protocol::write_frame(&mut host.send, config::RESPONSE_OK).await?;
    rterm_protocol::write_frame(&mut client.send, config::RESPONSE_OK).await?;
    eprintln!("paired host and client");

    let host_to_client = tokio::spawn(forward_frames(host.recv, client.send));
    let client_to_host = tokio::spawn(forward_frames(client.recv, host.send));
    let (host_to_client, client_to_host) = tokio::try_join!(host_to_client, client_to_host)?;
    host_to_client?;
    client_to_host?;

    eprintln!("pair closed");
    Ok(())
}

async fn forward_frames(mut recv: RecvStream, mut send: SendStream) -> Result<()> {
    while let Some(frame) = rterm_protocol::read_frame(&mut recv).await? {
        rterm_protocol::write_frame(&mut send, &frame).await?;
    }
    let _ = send.finish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_require_key_with_cert() {
        assert!(Args::try_parse_from([
            "rterm-relay",
            "--listen",
            "127.0.0.1:4433",
            "--cert",
            "cert.pem",
        ])
        .is_err());
    }

    #[test]
    fn args_require_cert_with_key() {
        assert!(Args::try_parse_from([
            "rterm-relay",
            "--listen",
            "127.0.0.1:4433",
            "--key",
            "key.pem",
        ])
        .is_err());
    }

    #[test]
    fn args_reject_token_and_token_file_together() {
        assert!(Args::try_parse_from([
            "rterm-relay",
            "--listen",
            "127.0.0.1:4433",
            "--token",
            "12345678901234567890123456789012",
            "--token-file",
            "token.txt",
        ])
        .is_err());
    }

    #[test]
    fn args_accept_listen_only() {
        assert!(Args::try_parse_from(["rterm-relay", "--listen", "127.0.0.1:4433"]).is_ok());
    }
}
