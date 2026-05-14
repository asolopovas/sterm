use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(long, conflicts_with = "relay", help = "Direct listen address")]
    pub listen: Option<SocketAddr>,

    #[arg(long, conflicts_with = "listen", help = "Public relay address")]
    pub relay: Option<String>,

    #[arg(
        long,
        requires = "listen",
        help = "UDP BitTorrent tracker for peer discovery"
    )]
    pub tracker: Option<String>,

    #[arg(long, default_value = rterm_protocol::DEFAULT_TRACKER_ROOM, help = "Shared tracker room name")]
    pub tracker_room: String,

    #[arg(long, conflicts_with = "token_file")]
    pub token: Option<String>,

    #[arg(long, env = "RTERM_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,

    #[arg(long, help = "Create a UPnP IGD UDP mapping for direct mode")]
    pub upnp: bool,

    #[arg(long, help = "External UDP port to request with UPnP")]
    pub upnp_external_port: Option<u16>,

    #[arg(
        long,
        requires = "relay",
        help = "SHA-256 fingerprint for a self-signed relay certificate"
    )]
    pub cert_sha256: Option<String>,

    #[arg(
        long,
        default_value_t = rterm_protocol::config::DEFAULT_UPNP_LEASE_SECONDS,
        help = "UPnP lease duration in seconds"
    )]
    pub upnp_lease_seconds: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_reject_listen_and_relay_together() {
        assert!(Args::try_parse_from([
            "rterm-host",
            "--listen",
            "127.0.0.1:4433",
            "--relay",
            "relay.example:443",
        ])
        .is_err());
    }

    #[test]
    fn args_require_listen_for_tracker() {
        assert!(Args::try_parse_from(["rterm-host", "--tracker", "127.0.0.1:6969"]).is_err());
    }

    #[test]
    fn args_require_relay_for_cert_pin() {
        assert!(Args::try_parse_from(["rterm-host", "--cert-sha256", &"01".repeat(32)]).is_err());
    }

    #[test]
    fn args_accept_direct_listen() {
        assert!(Args::try_parse_from(["rterm-host", "--listen", "127.0.0.1:4433"]).is_ok());
    }
}
