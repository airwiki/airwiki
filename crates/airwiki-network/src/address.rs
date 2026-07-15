//! Validated LAN dial addresses and volatile peer-address selection.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use thiserror::Error;

/// Maximum accepted size for an address pasted into the advanced LAN UI.
pub const MAX_MANUAL_LAN_ADDRESS_BYTES: usize = 512;

/// Maximum number of volatile peer identities retained from LAN discovery.
///
/// Trusted identities remain durable in SQLite; this limit applies only to the
/// unauthenticated, runtime-only address cache.
pub const MAX_VOLATILE_LAN_PEERS: usize = 256;

/// Maximum current mDNS observations retained for one peer identity.
pub const MAX_MDNS_ADDRESSES_PER_PEER: usize = 8;

/// A manually supplied, canonical TCP address that cannot leave the local network.
///
/// Accepted forms are `/ip4/<address>/tcp/<port>` and
/// `/ip6/<address>/tcp/<port>`, optionally followed by a terminal
/// `/p2p/<peer-id>`. DNS, UDP, QUIC, relay and globally routable addresses are
/// deliberately outside this type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManualLanAddress {
    address: Multiaddr,
    ip: IpAddr,
    peer_id: Option<PeerId>,
}

impl ManualLanAddress {
    /// Builds a dialable IPv4 address from a dynamic TCP listener and the
    /// authenticated identity shown by the advanced desktop fallback.
    ///
    /// The listener may be bound to `0.0.0.0`, but the published address always
    /// uses the concrete private/on-link address supplied by the desktop.
    pub fn from_ipv4_listener(
        listener: &Multiaddr,
        ip: Ipv4Addr,
        peer_id: PeerId,
    ) -> Result<Self, LanAddressError> {
        let mut protocols = listener.iter();
        if !matches!(protocols.next(), Some(Protocol::Ip4(_))) {
            return Err(LanAddressError::InvalidListener);
        }
        let Some(Protocol::Tcp(port)) = protocols.next() else {
            return Err(LanAddressError::InvalidListener);
        };
        if protocols.next().is_some() {
            return Err(LanAddressError::InvalidListener);
        }
        Self::from_str(&format!("/ip4/{ip}/tcp/{port}/p2p/{peer_id}"))
    }

    /// Returns the complete canonical multiaddress, including its optional PeerId.
    pub fn as_multiaddr(&self) -> &Multiaddr {
        &self.address
    }

    /// Returns the terminal PeerId when the input included one.
    pub fn peer_id(&self) -> Option<PeerId> {
        self.peer_id
    }

    /// Returns the canonical destination IP for platform route validation.
    pub fn ip_addr(&self) -> IpAddr {
        self.ip
    }

    /// Returns the canonical IP/TCP address without a terminal PeerId.
    ///
    /// `DialOpts::peer_id(...).addresses(...)` already carries the authenticated
    /// identity separately and expects transport addresses in this form.
    pub fn transport_address(&self) -> Multiaddr {
        let mut address = self.address.clone();
        if self.peer_id.is_some() {
            let _ = address.pop();
        }
        address
    }

    pub fn into_multiaddr(self) -> Multiaddr {
        self.address
    }
}

impl fmt::Display for ManualLanAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.address.fmt(formatter)
    }
}

impl FromStr for ManualLanAddress {
    type Err = LanAddressError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(LanAddressError::Empty);
        }
        if input.len() > MAX_MANUAL_LAN_ADDRESS_BYTES {
            return Err(LanAddressError::TooLong);
        }

        let parts = input.split('/').collect::<Vec<_>>();
        if parts.first() != Some(&"") || !matches!(parts.len(), 5 | 7) {
            return Err(LanAddressError::InvalidShape);
        }

        let ip = parse_lan_ip(parts[1], parts[2])?;
        if parts[3] != "tcp" {
            return Err(LanAddressError::UnsupportedTransport);
        }
        let port = parts[4]
            .parse::<u16>()
            .map_err(|_| LanAddressError::InvalidPort)?;
        if port == 0 {
            return Err(LanAddressError::ZeroPort);
        }

        let peer_id = if parts.len() == 7 {
            if parts[5] != "p2p" {
                return Err(LanAddressError::UnsupportedSuffix);
            }
            Some(PeerId::from_str(parts[6]).map_err(|_| LanAddressError::InvalidPeerId)?)
        } else {
            None
        };

        let mut address = Multiaddr::empty();
        let ip = match ip {
            LanIp::V4(ip) => {
                address.push(Protocol::Ip4(ip));
                IpAddr::V4(ip)
            }
            LanIp::V6(ip) => {
                address.push(Protocol::Ip6(ip));
                IpAddr::V6(ip)
            }
        };
        address.push(Protocol::Tcp(port));
        if let Some(peer) = peer_id {
            address.push(Protocol::P2p(peer));
        }

        Ok(Self {
            address,
            ip,
            peer_id,
        })
    }
}

impl TryFrom<&str> for ManualLanAddress {
    type Error = LanAddressError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        input.parse()
    }
}

impl TryFrom<Multiaddr> for ManualLanAddress {
    type Error = LanAddressError;

    fn try_from(address: Multiaddr) -> Result<Self, Self::Error> {
        Self::from_str(&address.to_string())
    }
}

impl From<ManualLanAddress> for Multiaddr {
    fn from(address: ManualLanAddress) -> Self {
        address.into_multiaddr()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LanAddressError {
    #[error("LAN address is empty")]
    Empty,
    #[error("LAN address exceeds the 512 byte limit")]
    TooLong,
    #[error("LAN address must contain exactly IP, TCP port and an optional terminal PeerId")]
    InvalidShape,
    #[error("only ip4 and ip6 LAN addresses are supported")]
    UnsupportedNetwork,
    #[error("IP address is invalid")]
    InvalidIp,
    #[error("IP address is not local or private")]
    NonLocalIp,
    #[error("only TCP LAN addresses are supported")]
    UnsupportedTransport,
    #[error("TCP port is invalid")]
    InvalidPort,
    #[error("TCP port zero is not dialable")]
    ZeroPort,
    #[error("only an optional terminal p2p component is supported")]
    UnsupportedSuffix,
    #[error("PeerId is invalid")]
    InvalidPeerId,
    #[error("listener address is not a plain IPv4 TCP endpoint")]
    InvalidListener,
    #[error("address belongs to a different PeerId")]
    PeerMismatch,
    #[error("volatile LAN address capacity is exhausted")]
    CapacityExceeded,
}

enum LanIp {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

fn parse_lan_ip(protocol: &str, value: &str) -> Result<LanIp, LanAddressError> {
    match protocol {
        "ip4" => {
            let ip = value
                .parse::<Ipv4Addr>()
                .map_err(|_| LanAddressError::InvalidIp)?;
            if !is_allowed_ipv4(ip) {
                return Err(LanAddressError::NonLocalIp);
            }
            Ok(LanIp::V4(ip))
        }
        "ip6" => {
            let ip = value
                .parse::<Ipv6Addr>()
                .map_err(|_| LanAddressError::InvalidIp)?;
            if !is_allowed_ipv6(ip) {
                return Err(LanAddressError::NonLocalIp);
            }
            Ok(LanIp::V6(ip))
        }
        _ => Err(LanAddressError::UnsupportedNetwork),
    }
}

fn is_allowed_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback() || ip.is_link_local()
}

fn is_allowed_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
}

/// Volatile addresses for one authenticated libp2p identity.
#[derive(Debug, Default)]
struct PeerAddresses {
    /// Most recently announced mDNS address first.
    mdns: Vec<Multiaddr>,
    /// Last address that completed an outbound Noise handshake.
    authenticated_outbound: Option<Multiaddr>,
}

/// In-memory LAN address source used for explicit, bounded redials.
///
/// mDNS addresses are current observations and therefore take precedence over
/// the last authenticated outbound address. Nothing in this type is serializable
/// or persisted; callers must clear a peer immediately when revoking it.
#[derive(Debug, Default)]
pub struct PeerAddressBook {
    peers: HashMap<PeerId, PeerAddresses>,
}

impl PeerAddressBook {
    /// Records a current mDNS address, moving a repeated address to the front.
    pub fn record_mdns(
        &mut self,
        peer: PeerId,
        address: Multiaddr,
    ) -> Result<Multiaddr, LanAddressError> {
        let address = normalize_for_peer(peer, address)?;
        if !self.peers.contains_key(&peer) && self.peers.len() >= MAX_VOLATILE_LAN_PEERS {
            return Err(LanAddressError::CapacityExceeded);
        }
        let addresses = &mut self.peers.entry(peer).or_default().mdns;
        addresses.retain(|candidate| candidate != &address);
        addresses.insert(0, address.clone());
        addresses.truncate(MAX_MDNS_ADDRESSES_PER_PEER);
        Ok(address)
    }

    /// Expires only the matching mDNS observation.
    ///
    /// An equal authenticated outbound fallback remains available because it was
    /// established by Noise rather than inferred from discovery.
    pub fn expire_mdns(
        &mut self,
        peer: PeerId,
        address: Multiaddr,
    ) -> Result<Option<Multiaddr>, LanAddressError> {
        let address = normalize_for_peer(peer, address)?;
        let Some(addresses) = self.peers.get_mut(&peer) else {
            return Ok(None);
        };
        let previous_len = addresses.mdns.len();
        addresses.mdns.retain(|candidate| candidate != &address);
        let removed = previous_len != addresses.mdns.len();
        self.remove_empty_peer(peer);
        Ok(removed.then_some(address))
    }

    /// Stores the last dial address that completed an outbound Noise handshake.
    pub fn record_authenticated_outbound(
        &mut self,
        peer: PeerId,
        address: Multiaddr,
    ) -> Result<Multiaddr, LanAddressError> {
        let address = normalize_for_peer(peer, address)?;
        if !self.peers.contains_key(&peer) && self.peers.len() >= MAX_VOLATILE_LAN_PEERS {
            let eviction = self
                .peers
                .iter()
                .filter(|(_, addresses)| addresses.authenticated_outbound.is_none())
                .map(|(candidate, _)| *candidate)
                .min_by(|left, right| left.to_bytes().cmp(&right.to_bytes()));
            if let Some(eviction) = eviction {
                self.peers.remove(&eviction);
            } else {
                return Err(LanAddressError::CapacityExceeded);
            }
        }
        self.peers.entry(peer).or_default().authenticated_outbound = Some(address.clone());
        Ok(address)
    }

    /// Returns current mDNS addresses followed by a distinct authenticated fallback.
    pub fn dial_addresses(&self, peer: &PeerId) -> Vec<Multiaddr> {
        let Some(addresses) = self.peers.get(peer) else {
            return Vec::new();
        };
        let mut ordered = addresses.mdns.clone();
        if let Some(authenticated) = &addresses.authenticated_outbound
            && !ordered.contains(authenticated)
        {
            ordered.push(authenticated.clone());
        }
        ordered
    }

    /// Removes all volatile addresses associated with a revoked or blocked peer.
    pub fn clear_peer(&mut self, peer: &PeerId) -> bool {
        self.peers.remove(peer).is_some()
    }

    pub fn clear(&mut self) {
        self.peers.clear();
    }

    fn remove_empty_peer(&mut self, peer: PeerId) {
        let should_remove = self.peers.get(&peer).is_some_and(|addresses| {
            addresses.mdns.is_empty() && addresses.authenticated_outbound.is_none()
        });
        if should_remove {
            self.peers.remove(&peer);
        }
    }
}

fn normalize_for_peer(
    expected_peer: PeerId,
    address: Multiaddr,
) -> Result<Multiaddr, LanAddressError> {
    let parsed = ManualLanAddress::from_str(&address.to_string())?;
    if parsed
        .peer_id()
        .is_some_and(|address_peer| address_peer != expected_peer)
    {
        return Err(LanAddressError::PeerMismatch);
    }
    Ok(parsed.transport_address())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(input: &str) -> Multiaddr {
        input.parse().unwrap()
    }

    #[test]
    fn manual_address_accepts_private_ipv4() {
        let parsed = ManualLanAddress::from_str("/ip4/192.168.1.25/tcp/61743").unwrap();

        assert_eq!(parsed.to_string(), "/ip4/192.168.1.25/tcp/61743");
        assert_eq!(parsed.ip_addr(), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 25)));
    }

    #[test]
    fn manual_address_accepts_ipv4_loopback() {
        let result = ManualLanAddress::from_str("/ip4/127.0.0.1/tcp/43123");

        assert!(result.is_ok(), "unexpected parse error: {result:?}");
    }

    #[test]
    fn manual_address_accepts_ipv4_link_local() {
        let result = ManualLanAddress::from_str("/ip4/169.254.12.34/tcp/8080");

        assert!(result.is_ok(), "unexpected parse error: {result:?}");
    }

    #[test]
    fn manual_address_canonicalizes_unique_local_ipv6() {
        let parsed = ManualLanAddress::from_str("/ip6/FD00:0:0:0:0:0:0:1/tcp/8080").unwrap();

        assert_eq!(parsed.to_string(), "/ip6/fd00::1/tcp/8080");
    }

    #[test]
    fn manual_address_accepts_ipv6_link_local() {
        let result = ManualLanAddress::from_str("/ip6/fe80::1/tcp/8080");

        assert!(result.is_ok(), "unexpected parse error: {result:?}");
    }

    #[test]
    fn manual_address_preserves_valid_terminal_peer_id() {
        let peer = PeerId::random();
        let input = format!("/ip4/10.0.0.8/tcp/8080/p2p/{peer}");

        let parsed = ManualLanAddress::from_str(&input).unwrap();

        assert_eq!(parsed.peer_id(), Some(peer));
    }

    #[test]
    fn manual_address_builds_dialable_fallback_from_dynamic_listener() {
        let peer = PeerId::random();
        let listener = address("/ip4/0.0.0.0/tcp/61743");

        let parsed =
            ManualLanAddress::from_ipv4_listener(&listener, Ipv4Addr::new(192, 168, 1, 25), peer)
                .unwrap();

        assert_eq!(
            parsed.to_string(),
            format!("/ip4/192.168.1.25/tcp/61743/p2p/{peer}")
        );
    }

    #[test]
    fn manual_address_rejects_non_tcp_listener_fallback() {
        let listener = address("/ip4/0.0.0.0/udp/5353");

        let error = ManualLanAddress::from_ipv4_listener(
            &listener,
            Ipv4Addr::new(192, 168, 1, 25),
            PeerId::random(),
        )
        .unwrap_err();

        assert_eq!(error, LanAddressError::InvalidListener);
    }

    #[test]
    fn manual_address_rejects_global_ipv4() {
        let error = ManualLanAddress::from_str("/ip4/8.8.8.8/tcp/443").unwrap_err();

        assert_eq!(error, LanAddressError::NonLocalIp);
    }

    #[test]
    fn manual_address_rejects_unspecified_ipv4() {
        let error = ManualLanAddress::from_str("/ip4/0.0.0.0/tcp/443").unwrap_err();

        assert_eq!(error, LanAddressError::NonLocalIp);
    }

    #[test]
    fn manual_address_rejects_global_ipv6() {
        let error = ManualLanAddress::from_str("/ip6/2001:4860:4860::8888/tcp/443").unwrap_err();

        assert_eq!(error, LanAddressError::NonLocalIp);
    }

    #[test]
    fn manual_address_rejects_dns() {
        let error = ManualLanAddress::from_str("/dns4/example.com/tcp/443").unwrap_err();

        assert_eq!(error, LanAddressError::UnsupportedNetwork);
    }

    #[test]
    fn manual_address_rejects_udp() {
        let error = ManualLanAddress::from_str("/ip4/192.168.1.25/udp/5353").unwrap_err();

        assert_eq!(error, LanAddressError::UnsupportedTransport);
    }

    #[test]
    fn manual_address_rejects_quic_suffix() {
        let error = ManualLanAddress::from_str("/ip4/192.168.1.25/tcp/443/quic-v1").unwrap_err();

        assert_eq!(error, LanAddressError::InvalidShape);
    }

    #[test]
    fn manual_address_rejects_relay_suffix() {
        let error =
            ManualLanAddress::from_str("/ip4/192.168.1.25/tcp/443/p2p-circuit/p2p/12D3KooWInvalid")
                .unwrap_err();

        assert_eq!(error, LanAddressError::InvalidShape);
    }

    #[test]
    fn manual_address_rejects_zero_port() {
        let error = ManualLanAddress::from_str("/ip4/192.168.1.25/tcp/0").unwrap_err();

        assert_eq!(error, LanAddressError::ZeroPort);
    }

    #[test]
    fn manual_address_rejects_invalid_peer_id() {
        let error = ManualLanAddress::from_str("/ip4/192.168.1.25/tcp/443/p2p/not-a-valid-peer-id")
            .unwrap_err();

        assert_eq!(error, LanAddressError::InvalidPeerId);
    }

    #[test]
    fn manual_address_rejects_input_larger_than_limit() {
        let input = format!("/ip4/192.168.1.25/tcp/443/{}", "x".repeat(512));

        let error = ManualLanAddress::from_str(&input).unwrap_err();

        assert_eq!(error, LanAddressError::TooLong);
    }

    #[test]
    fn address_book_prioritizes_newest_mdns_port() {
        let peer = PeerId::random();
        let mut book = PeerAddressBook::default();
        book.record_mdns(peer, address("/ip4/192.168.1.25/tcp/41000"))
            .unwrap();
        book.record_mdns(peer, address("/ip4/192.168.1.25/tcp/42000"))
            .unwrap();

        let ordered = book.dial_addresses(&peer);

        assert_eq!(ordered[0], address("/ip4/192.168.1.25/tcp/42000"));
    }

    #[test]
    fn address_book_uses_authenticated_address_as_fallback() {
        let peer = PeerId::random();
        let mut book = PeerAddressBook::default();
        book.record_authenticated_outbound(peer, address("/ip4/10.0.0.5/tcp/40000"))
            .unwrap();
        book.record_mdns(peer, address("/ip4/10.0.0.5/tcp/41000"))
            .unwrap();

        let ordered = book.dial_addresses(&peer);

        assert_eq!(
            ordered,
            [
                address("/ip4/10.0.0.5/tcp/41000"),
                address("/ip4/10.0.0.5/tcp/40000"),
            ]
        );
    }

    #[test]
    fn address_book_expiry_preserves_authenticated_fallback() {
        let peer = PeerId::random();
        let current = address("/ip4/10.0.0.5/tcp/41000");
        let mut book = PeerAddressBook::default();
        book.record_authenticated_outbound(peer, current.clone())
            .unwrap();
        book.record_mdns(peer, current.clone()).unwrap();

        let removed = book.expire_mdns(peer, current.clone()).unwrap();

        assert_eq!(removed, Some(current.clone()));
        assert_eq!(book.dial_addresses(&peer), [current]);
    }

    #[test]
    fn address_book_expiry_removes_address_without_authenticated_fallback() {
        let peer = PeerId::random();
        let current = address("/ip4/10.0.0.5/tcp/41000");
        let mut book = PeerAddressBook::default();
        book.record_mdns(peer, current.clone()).unwrap();

        book.expire_mdns(peer, current).unwrap();

        assert!(book.dial_addresses(&peer).is_empty());
    }

    #[test]
    fn address_book_normalizes_matching_terminal_peer_id() {
        let peer = PeerId::random();
        let mut book = PeerAddressBook::default();
        let announced = address(&format!("/ip4/10.0.0.5/tcp/41000/p2p/{peer}"));

        book.record_mdns(peer, announced).unwrap();

        assert_eq!(
            book.dial_addresses(&peer),
            [address("/ip4/10.0.0.5/tcp/41000")]
        );
    }

    #[test]
    fn address_book_rejects_mismatched_terminal_peer_id() {
        let expected_peer = PeerId::random();
        let other_peer = PeerId::random();
        let mut book = PeerAddressBook::default();
        let announced = address(&format!("/ip4/10.0.0.5/tcp/41000/p2p/{other_peer}"));

        let error = book.record_mdns(expected_peer, announced).unwrap_err();

        assert_eq!(error, LanAddressError::PeerMismatch);
    }

    #[test]
    fn address_book_clear_peer_removes_discovery_and_fallback() {
        let peer = PeerId::random();
        let mut book = PeerAddressBook::default();
        book.record_mdns(peer, address("/ip4/192.168.1.25/tcp/41000"))
            .unwrap();
        book.record_authenticated_outbound(peer, address("/ip4/192.168.1.25/tcp/40000"))
            .unwrap();

        let removed = book.clear_peer(&peer);

        assert!(removed);
        assert!(book.dial_addresses(&peer).is_empty());
    }

    #[test]
    fn address_book_retains_only_bounded_mdns_addresses_per_peer() {
        let peer = PeerId::random();
        let mut book = PeerAddressBook::default();
        for offset in 0..=MAX_MDNS_ADDRESSES_PER_PEER {
            let port = 40_000 + u16::try_from(offset).unwrap();
            book.record_mdns(peer, address(&format!("/ip4/10.0.0.5/tcp/{port}")))
                .unwrap();
        }

        let retained = book.dial_addresses(&peer);

        assert_eq!(retained.len(), MAX_MDNS_ADDRESSES_PER_PEER);
        assert_eq!(
            retained[0],
            address(&format!(
                "/ip4/10.0.0.5/tcp/{}",
                40_000 + u16::try_from(MAX_MDNS_ADDRESSES_PER_PEER).unwrap()
            ))
        );
    }

    #[test]
    fn address_book_rejects_new_peer_after_volatile_capacity() {
        let mut book = PeerAddressBook::default();
        for _ in 0..MAX_VOLATILE_LAN_PEERS {
            book.record_mdns(PeerId::random(), address("/ip4/10.0.0.5/tcp/41000"))
                .unwrap();
        }

        let error = book
            .record_mdns(PeerId::random(), address("/ip4/10.0.0.6/tcp/41000"))
            .unwrap_err();

        assert_eq!(error, LanAddressError::CapacityExceeded);
    }

    #[test]
    fn address_book_still_refreshes_known_peer_at_capacity() {
        let mut book = PeerAddressBook::default();
        let known = PeerId::random();
        book.record_mdns(known, address("/ip4/10.0.0.5/tcp/41000"))
            .unwrap();
        for _ in 1..MAX_VOLATILE_LAN_PEERS {
            book.record_mdns(PeerId::random(), address("/ip4/10.0.0.5/tcp/41000"))
                .unwrap();
        }

        let result = book.record_mdns(known, address("/ip4/10.0.0.5/tcp/42000"));

        assert!(result.is_ok(), "known peer refresh failed: {result:?}");
    }

    #[test]
    fn authenticated_address_replaces_unauthenticated_entry_at_capacity() {
        let mut book = PeerAddressBook::default();
        for _ in 0..MAX_VOLATILE_LAN_PEERS {
            book.record_mdns(PeerId::random(), address("/ip4/10.0.0.5/tcp/41000"))
                .unwrap();
        }
        let authenticated = PeerId::random();

        let result =
            book.record_authenticated_outbound(authenticated, address("/ip4/10.0.0.6/tcp/42000"));

        assert!(
            result.is_ok(),
            "authenticated address was rejected: {result:?}"
        );
        assert_eq!(book.peers.len(), MAX_VOLATILE_LAN_PEERS);
        assert_eq!(
            book.dial_addresses(&authenticated),
            [address("/ip4/10.0.0.6/tcp/42000")]
        );
    }

    #[test]
    fn authenticated_address_fails_only_when_all_capacity_is_authenticated() {
        let mut book = PeerAddressBook::default();
        for _ in 0..MAX_VOLATILE_LAN_PEERS {
            book.record_authenticated_outbound(
                PeerId::random(),
                address("/ip4/10.0.0.5/tcp/41000"),
            )
            .unwrap();
        }

        let error = book
            .record_authenticated_outbound(PeerId::random(), address("/ip4/10.0.0.6/tcp/42000"))
            .unwrap_err();

        assert_eq!(error, LanAddressError::CapacityExceeded);
    }
}
