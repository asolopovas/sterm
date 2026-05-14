use anyhow::{Context, Result};
use quinn::Endpoint;
use rterm_protocol::config;
use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use sterm_lib::{pty, PairingPayload, TerminalShell};
use tokio::sync::Semaphore;

const DEFAULT_TRACKER: &str = "udp://tracker.opentrackr.org:1337";
const DEFAULT_ROOM: &str = "sterm";

#[tokio::main]
async fn main() -> Result<()> {
    rterm_protocol::install_crypto_provider();
    let args = Args::parse()?;
    let token = args.token.unwrap_or_else(new_token);
    let generated = rterm_protocol::self_signed_server_config(vec![
        config::DEFAULT_DIRECT_SERVER_NAME.to_string(),
    ])?;
    let endpoint = Endpoint::server(generated.config, args.listen)?;
    let listen = endpoint.local_addr()?;
    let public_ip = sterm_lib::p2p_probe::discover_public_ip_via_stun()
        .await
        .ok();
    let host_addr = SocketAddr::new(public_ip.unwrap_or(args.host_ip), listen.port());

    let pairing = PairingPayload {
        v: 1,
        mode: "tracker".to_string(),
        host: Some(host_addr.to_string()),
        token: token.clone(),
        cert_sha256: generated.certificate_sha256_pin.clone(),
        tracker: Some(args.tracker.clone()),
        tracker_room: Some(args.room.clone()),
        relay: None,
        relay_cert_sha256: None,
        requires_password: Some(args.password.is_some()),
        password: None,
    };
    let pairing_json = compact_pairing_json(&pairing)?;

    println!("sterm host listening on udp://{listen}");
    println!("public/direct address in QR: {host_addr}");
    println!("shell: {:?}", args.shell);
    println!("password required: {}", args.password.is_some());
    println!("certificate pin: {}", generated.certificate_sha256_pin);
    println!("pairing JSON: {pairing_json}\n");
    print_qr(&pairing_json)?;

    spawn_tracker(
        args.tracker.clone(),
        args.room.clone(),
        token.clone(),
        listen.port(),
    );

    let limit = Arc::new(Semaphore::new(config::DIRECT_HOST_CONNECTION_LIMIT));
    while let Some(connecting) = endpoint.accept().await {
        let Ok(permit) = limit.clone().try_acquire_owned() else {
            eprintln!("connection limit reached; dropping client");
            continue;
        };
        let token = token.clone();
        let password = args.password.clone();
        let shell = args.shell.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(err) = handle_connection(connecting, token, password, shell).await {
                eprintln!("connection ended: {err:#}");
            }
        });
    }
    Ok(())
}

async fn handle_connection(
    connecting: quinn::Incoming,
    token: String,
    password: Option<String>,
    shell: TerminalShell,
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
        rterm_protocol::write_frame(&mut send, config::RESPONSE_ERR_BAD_HELLO).await?;
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
    let secret = auth_secret(&token, password.as_deref());
    match rterm_protocol::verify_auth_response(received.as_ref(), secret.as_bytes(), &nonce) {
        Ok(role) if role == config::ROLE_CLIENT => {}
        _ => {
            rterm_protocol::write_frame(&mut send, config::RESPONSE_ERR_AUTH_FAILED).await?;
            return Ok(());
        }
    }
    rterm_protocol::write_frame(&mut send, config::RESPONSE_OK).await?;
    pty::run_pty(send, recv, shell).await?;
    Ok(())
}

fn spawn_tracker(tracker: String, room: String, token: String, port: u16) {
    tokio::spawn(async move {
        let Ok(target) = rterm_protocol::resolve_tracker(&tracker) else {
            eprintln!("tracker resolve failed: {tracker}");
            return;
        };
        loop {
            match rterm_protocol::announce_to_tracker(&target, token.as_bytes(), &room, port).await
            {
                Ok(announce) => {
                    eprintln!(
                        "announced to tracker {}; {} peer(s), next in {:?}",
                        target.addr,
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
    });
}

#[derive(Clone)]
struct Args {
    listen: SocketAddr,
    host_ip: IpAddr,
    tracker: String,
    room: String,
    token: Option<String>,
    password: Option<String>,
    shell: TerminalShell,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut args = env::args().skip(1);
        let mut out = Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4433),
            host_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            tracker: DEFAULT_TRACKER.to_string(),
            room: DEFAULT_ROOM.to_string(),
            token: None,
            password: None,
            shell: TerminalShell::PowerShell,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--listen" => out.listen = args.next().context("--listen value")?.parse()?,
                "--host-ip" => out.host_ip = args.next().context("--host-ip value")?.parse()?,
                "--tracker" => out.tracker = args.next().context("--tracker value")?,
                "--room" => out.room = args.next().context("--room value")?,
                "--token" => out.token = Some(args.next().context("--token value")?),
                "--password" => out.password = Some(args.next().context("--password value")?),
                "--shell" => out.shell = parse_shell(&args.next().context("--shell value")?)?,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => anyhow::bail!("unknown argument: {other}"),
            }
        }
        Ok(out)
    }
}

fn parse_shell(value: &str) -> Result<TerminalShell> {
    match value.to_ascii_lowercase().as_str() {
        "cmd" => Ok(TerminalShell::Cmd),
        "powershell" | "pwsh" | "ps" => Ok(TerminalShell::PowerShell),
        "wsl" => Ok(TerminalShell::Wsl),
        _ => anyhow::bail!("--shell must be cmd, powershell, or wsl"),
    }
}

fn print_help() {
    println!(
        "Usage: sterm-host [--listen 0.0.0.0:4433] [--shell powershell|cmd|wsl] [--password SECRET]"
    );
}

fn new_token() -> String {
    use base64::{engine::general_purpose, Engine as _};
    use rand::{rngs::OsRng, RngCore};
    let mut bytes = [0u8; 48];
    OsRng.fill_bytes(&mut bytes);
    general_purpose::STANDARD_NO_PAD.encode(bytes)
}

fn auth_secret(token: &str, password: Option<&str>) -> String {
    match password.filter(|value| !value.is_empty()) {
        Some(password) => format!("{token}\0{password}"),
        None => token.to_string(),
    }
}

fn compact_pairing_json(pairing: &PairingPayload) -> Result<String> {
    let mut value = serde_json::json!({
        "v": pairing.v,
        "m": "t",
        "h": pairing.host,
        "k": pairing.token,
        "c": pairing.cert_sha256,
        "r": pairing.tracker,
        "q": pairing.tracker_room,
    });
    if pairing.requires_password.unwrap_or(false) {
        value["a"] = serde_json::Value::Bool(true);
    }
    Ok(serde_json::to_string(&value)?)
}

fn print_qr(data: &str) -> Result<()> {
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L)?;
    let image = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .build();
    println!("{image}");
    Ok(())
}
