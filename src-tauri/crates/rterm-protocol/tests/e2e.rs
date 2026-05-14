use anyhow::{Context, Result};
use quinn::Endpoint;
use rterm_protocol::config;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

const GOOD_TOKEN: &str = "abcdefghijklmnopqrstuvwxyz123456";
const BAD_TOKEN: &str = "12345678901234567890123456789012";
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn direct_quic_authenticates_and_transfers_frames() -> Result<()> {
    rterm_protocol::install_crypto_provider();
    let generated = rterm_protocol::self_signed_server_config(vec![
        config::DEFAULT_DIRECT_SERVER_NAME.to_owned(),
    ])?;
    let expected_sha256 =
        rterm_protocol::parse_sha256_fingerprint(&generated.certificate_sha256_pin)?;
    let server = Endpoint::server(generated.config, local_udp_addr())?;
    let server_addr = server.local_addr()?;

    let server_task = tokio::spawn(async move {
        let accepted = accept_client(&server, GOOD_TOKEN).await?;
        let (mut send, mut recv) = accepted.streams;
        rterm_protocol::write_frame(&mut send, config::RESPONSE_OK).await?;
        let frame = rterm_protocol::read_frame(&mut recv)
            .await?
            .context("client closed before payload")?;
        rterm_protocol::write_frame(&mut send, &frame).await?;
        wait_for_close(&accepted.connection).await?;
        Ok::<_, anyhow::Error>(())
    });

    let mut client = Endpoint::client(local_udp_addr())?;
    client.set_default_client_config(rterm_protocol::pinned_client_config(expected_sha256)?);
    let connection = client
        .connect(server_addr, config::DEFAULT_DIRECT_SERVER_NAME)?
        .await?;
    let (mut send, mut recv) = connection.open_bi().await?;

    rterm_protocol::write_frame(&mut send, config::HELLO_CLIENT).await?;
    let nonce = read_challenge(&mut recv).await?;
    let auth =
        rterm_protocol::auth_response_frame(config::ROLE_CLIENT, GOOD_TOKEN.as_bytes(), &nonce)?;
    rterm_protocol::write_frame(&mut send, &auth).await?;
    let response = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("server closed before auth response")?;
    assert_eq!(response.as_ref(), config::RESPONSE_OK);

    rterm_protocol::write_frame(&mut send, b"end-to-end payload").await?;
    let echo = rterm_protocol::read_frame(&mut recv)
        .await?
        .context("server closed before echo")?;
    assert_eq!(echo.as_ref(), b"end-to-end payload");

    connection.close(config::CLOSE_NORMAL.into(), config::CLOSE_REASON_DONE);
    client.wait_idle().await;
    tokio::time::timeout(TEST_TIMEOUT, server_task)
        .await
        .context("server task timed out")???;
    Ok(())
}

#[tokio::test]
async fn direct_quic_rejects_bad_token_before_payload() -> Result<()> {
    rterm_protocol::install_crypto_provider();
    let generated = rterm_protocol::self_signed_server_config(vec![
        config::DEFAULT_DIRECT_SERVER_NAME.to_owned(),
    ])?;
    let expected_sha256 =
        rterm_protocol::parse_sha256_fingerprint(&generated.certificate_sha256_pin)?;
    let server = Endpoint::server(generated.config, local_udp_addr())?;
    let server_addr = server.local_addr()?;

    let server_task = tokio::spawn(async move {
        let connection = server
            .accept()
            .await
            .context("server accept ended")?
            .await?;
        let (mut send, mut recv) = connection.accept_bi().await?;
        let hello = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
            .await?
            .context("client closed before hello")?;
        assert_eq!(hello.as_ref(), config::HELLO_CLIENT);
        let nonce = rterm_protocol::new_auth_challenge()?;
        rterm_protocol::write_frame(&mut send, &rterm_protocol::challenge_frame(&nonce)).await?;
        let auth = rterm_protocol::read_frame_limited(&mut recv, config::AUTH_FRAME_LIMIT)
            .await?
            .context("client closed before auth")?;
        assert!(
            rterm_protocol::verify_auth_response(auth.as_ref(), GOOD_TOKEN.as_bytes(), &nonce)
                .is_err()
        );
        rterm_protocol::write_frame(&mut send, config::RESPONSE_ERR_AUTH_FAILED).await?;
        wait_for_close(&connection).await?;
        Ok::<_, anyhow::Error>(())
    });

    let mut client = Endpoint::client(local_udp_addr())?;
    client.set_default_client_config(rterm_protocol::pinned_client_config(expected_sha256)?);
    let connection = client
        .connect(server_addr, config::DEFAULT_DIRECT_SERVER_NAME)?
        .await?;
    let (mut send, mut recv) = connection.open_bi().await?;

    rterm_protocol::write_frame(&mut send, config::HELLO_CLIENT).await?;
    let nonce = read_challenge(&mut recv).await?;
    let auth =
        rterm_protocol::auth_response_frame(config::ROLE_CLIENT, BAD_TOKEN.as_bytes(), &nonce)?;
    rterm_protocol::write_frame(&mut send, &auth).await?;
    let response = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("server closed before rejection")?;
    assert_eq!(response.as_ref(), config::RESPONSE_ERR_AUTH_FAILED);

    connection.close(
        config::CLOSE_AUTH_FAILED.into(),
        config::CLOSE_REASON_AUTH_FAILED,
    );
    client.wait_idle().await;
    tokio::time::timeout(TEST_TIMEOUT, server_task)
        .await
        .context("server task timed out")???;
    Ok(())
}

struct AcceptedClient {
    connection: quinn::Connection,
    streams: (quinn::SendStream, quinn::RecvStream),
}

async fn accept_client(endpoint: &Endpoint, token: &str) -> Result<AcceptedClient> {
    let connection = endpoint
        .accept()
        .await
        .context("server accept ended")?
        .await?;
    let (mut send, mut recv) = connection.accept_bi().await?;
    let hello = rterm_protocol::read_frame_limited(&mut recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("client closed before hello")?;
    assert_eq!(hello.as_ref(), config::HELLO_CLIENT);

    let nonce = rterm_protocol::new_auth_challenge()?;
    rterm_protocol::write_frame(&mut send, &rterm_protocol::challenge_frame(&nonce)).await?;
    let auth = rterm_protocol::read_frame_limited(&mut recv, config::AUTH_FRAME_LIMIT)
        .await?
        .context("client closed before auth")?;
    assert_eq!(
        rterm_protocol::verify_auth_response(auth.as_ref(), token.as_bytes(), &nonce)?,
        config::ROLE_CLIENT
    );
    Ok(AcceptedClient {
        connection,
        streams: (send, recv),
    })
}

async fn read_challenge(recv: &mut quinn::RecvStream) -> Result<[u8; config::NONCE_LEN]> {
    let challenge = rterm_protocol::read_frame_limited(recv, config::CONTROL_FRAME_LIMIT)
        .await?
        .context("server closed before challenge")?;
    rterm_protocol::parse_challenge_frame(&challenge)
}

async fn wait_for_close(connection: &quinn::Connection) -> Result<()> {
    tokio::time::timeout(TEST_TIMEOUT, connection.closed())
        .await
        .context("connection close timed out")?;
    Ok(())
}

fn local_udp_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
}
