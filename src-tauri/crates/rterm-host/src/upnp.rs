use anyhow::{Context, Result};
use igd::{aio::search_gateway, PortMappingProtocol, SearchOptions};
use rterm_protocol::config;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

pub struct UpnpMapping {
    gateway: igd::aio::Gateway,
    external_port: u16,
    external_ip: Ipv4Addr,
}

impl UpnpMapping {
    pub fn external_port(&self) -> u16 {
        self.external_port
    }

    pub fn external_ip(&self) -> Ipv4Addr {
        self.external_ip
    }
}

impl UpnpMapping {
    pub async fn remove(self) {
        match self
            .gateway
            .remove_port(PortMappingProtocol::UDP, self.external_port)
            .await
        {
            Ok(()) => eprintln!(
                "removed UPnP UDP mapping on external port {}",
                self.external_port
            ),
            Err(err) => eprintln!("failed to remove UPnP mapping: {err}"),
        }
    }
}

pub async fn add_upnp_mapping(
    listen: SocketAddr,
    external_port: u16,
    lease_seconds: u32,
) -> Result<UpnpMapping> {
    let local_ip = match listen.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => ip,
        IpAddr::V4(_) => local_lan_ipv4().context("detect local LAN IPv4 for UPnP")?,
        IpAddr::V6(_) => anyhow::bail!("UPnP IGD only supports IPv4 in this PoC"),
    };

    let local_addr = SocketAddrV4::new(local_ip, listen.port());
    eprintln!("searching for UPnP IGD gateway...");
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

    eprintln!(
        "UPnP mapped udp://{}:{} -> udp://{} for {}s",
        external_ip, external_port, local_addr, lease_seconds
    );

    Ok(UpnpMapping {
        gateway,
        external_port,
        external_ip,
    })
}

fn local_lan_ipv4() -> Result<Ipv4Addr> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.connect((
        Ipv4Addr::from(config::UPNP_PROBE_IPV4),
        config::UPNP_PROBE_PORT,
    ))?;
    match sock.local_addr()?.ip() {
        IpAddr::V4(ip) => Ok(ip),
        IpAddr::V6(_) => anyhow::bail!("could not determine local IPv4"),
    }
}
