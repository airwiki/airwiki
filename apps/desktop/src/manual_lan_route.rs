//! Platform validation for the advanced manual LAN connection flow.
//!
//! Parsing a private IP is not sufficient: RFC1918 and ULA ranges may belong
//! to an unrelated network. Windows therefore verifies that the destination is
//! covered by an active, on-link prefix immediately before dialing it.

use std::net::{IpAddr, Ipv4Addr};

use airwiki_network::{LanAddressError, ManualLanAddress, Multiaddr, PeerId};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum ManualLanRouteError {
    #[error("la conexión manual IPv6 todavía no está disponible; usa una dirección IPv4 local")]
    UnsupportedIpv6,
    #[cfg(any(target_os = "windows", test))]
    #[error("la dirección no pertenece a una subred local activa de este equipo")]
    NotOnActiveSubnet,
    #[cfg(any(target_os = "windows", test))]
    #[error(
        "la conexión IPv6 local necesita identificar la interfaz; usa el descubrimiento automático"
    )]
    ScopedIpv6Required,
    #[error("el sistema no pudo comprobar las direcciones locales activas")]
    InspectionFailed,
    #[error("no se pudo construir una dirección manual desde el listener LAN")]
    InvalidListener(#[from] LanAddressError),
}

/// Validates the route without exposing interface names or local addresses.
///
/// This operation is blocking on Windows and must run behind `spawn_blocking`.
pub(crate) fn validate(address: &ManualLanAddress) -> Result<(), ManualLanRouteError> {
    if !matches!(address.ip_addr(), IpAddr::V4(_)) {
        return Err(ManualLanRouteError::UnsupportedIpv6);
    }
    platform::validate(address)
}

/// Produces concrete, copyable IPv4 fallbacks for a dynamic listener.
///
/// Adapter enumeration is blocking on Windows, so callers must run this behind
/// the desktop worker's `spawn_blocking` boundary.
pub(crate) fn advertised_addresses(
    listener: &Multiaddr,
    peer_id: PeerId,
) -> Result<Vec<String>, ManualLanRouteError> {
    build_advertised_addresses(listener, peer_id, platform::active_ipv4_addresses()?)
}

fn build_advertised_addresses(
    listener: &Multiaddr,
    peer_id: PeerId,
    mut addresses: Vec<Ipv4Addr>,
) -> Result<Vec<String>, ManualLanRouteError> {
    addresses.retain(|address| {
        !address.is_unspecified()
            && !address.is_loopback()
            && (address.is_private() || address.is_link_local())
    });
    addresses.sort_unstable();
    addresses.dedup();
    addresses
        .into_iter()
        .map(|address| {
            ManualLanAddress::from_ipv4_listener(listener, address, peer_id)
                .map(|address| address.to_string())
                .map_err(Into::into)
        })
        .collect()
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveSubnet {
    local_address: IpAddr,
    prefix_length: u8,
}

#[cfg(any(target_os = "windows", test))]
fn validate_against_subnets(
    target: IpAddr,
    subnets: &[ActiveSubnet],
) -> Result<(), ManualLanRouteError> {
    if target.is_loopback() {
        return Ok(());
    }
    if matches!(target, IpAddr::V6(address) if address.is_unicast_link_local()) {
        // A bare fe80:: address is ambiguous when multiple interfaces share the
        // link-local prefix. ManualLanAddress intentionally has no zone syntax.
        return Err(ManualLanRouteError::ScopedIpv6Required);
    }
    if subnets.iter().any(|subnet| subnet.contains(target)) {
        Ok(())
    } else {
        Err(ManualLanRouteError::NotOnActiveSubnet)
    }
}

#[cfg(any(target_os = "windows", test))]
impl ActiveSubnet {
    fn contains(self, target: IpAddr) -> bool {
        match (self.local_address, target) {
            (IpAddr::V4(local), IpAddr::V4(target)) if self.prefix_length <= 32 => {
                let mask = prefix_mask_v4(self.prefix_length);
                u32::from(local) & mask == u32::from(target) & mask
            }
            (IpAddr::V6(local), IpAddr::V6(target)) if self.prefix_length <= 128 => {
                let mask = prefix_mask_v6(self.prefix_length);
                u128::from(local) & mask == u128::from(target) & mask
            }
            _ => false,
        }
    }
}

#[cfg(any(target_os = "windows", test))]
const fn prefix_mask_v4(prefix_length: u8) -> u32 {
    if prefix_length == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_length)
    }
}

#[cfg(any(target_os = "windows", test))]
const fn prefix_mask_v6(prefix_length: u8) -> u128 {
    if prefix_length == 0 {
        0
    } else {
        u128::MAX << (128 - prefix_length)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
        ptr::NonNull,
        slice,
    };

    use airwiki_network::ManualLanAddress;
    use windows::Win32::{
        Foundation::ERROR_SUCCESS,
        NetworkManagement::{
            IpHelper::{
                FreeMibTable, GetIfEntry2, GetUnicastIpAddressTable, IF_TYPE_SOFTWARE_LOOPBACK,
                IF_TYPE_TUNNEL, MIB_IF_ROW2, MIB_IF_TYPE_PPP, MIB_UNICASTIPADDRESS_ROW,
                MIB_UNICASTIPADDRESS_TABLE,
            },
            Ndis::IfOperStatusUp,
        },
        Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, IpDadStatePreferred, SOCKADDR_INET},
    };

    use super::{ActiveSubnet, ManualLanRouteError, validate_against_subnets};

    const MAX_UNICAST_ROWS: usize = 4_096;

    pub(super) fn validate(address: &ManualLanAddress) -> Result<(), ManualLanRouteError> {
        let subnets = active_subnets()?;
        validate_against_subnets(address.ip_addr(), &subnets)
    }

    pub(super) fn active_ipv4_addresses() -> Result<Vec<Ipv4Addr>, ManualLanRouteError> {
        Ok(active_subnets()?
            .into_iter()
            .filter_map(|subnet| match subnet.local_address {
                IpAddr::V4(address) => Some(address),
                IpAddr::V6(_) => None,
            })
            .collect())
    }

    fn active_subnets() -> Result<Vec<ActiveSubnet>, ManualLanRouteError> {
        let table = OwnedUnicastTable::load()?;
        let rows = table.rows()?;
        let mut interface_cache = HashMap::<u32, bool>::new();
        let mut subnets = Vec::with_capacity(rows.len().min(64));
        for row in rows {
            if row.DadState != IpDadStatePreferred {
                continue;
            }
            let interface_active = *interface_cache
                .entry(row.InterfaceIndex)
                .or_insert_with(|| interface_is_active(row));
            if !interface_active {
                continue;
            }
            let Some(local_address) = socket_address_to_ip(&row.Address) else {
                continue;
            };
            if !is_local_candidate(local_address)
                || !prefix_is_valid(local_address, row.OnLinkPrefixLength)
            {
                continue;
            }
            subnets.push(ActiveSubnet {
                local_address,
                prefix_length: row.OnLinkPrefixLength,
            });
        }
        Ok(subnets)
    }

    fn interface_is_active(address: &MIB_UNICASTIPADDRESS_ROW) -> bool {
        let mut interface = MIB_IF_ROW2 {
            InterfaceLuid: address.InterfaceLuid,
            InterfaceIndex: address.InterfaceIndex,
            ..Default::default()
        };
        // SAFETY: `interface` is initialized with the identity fields required
        // by GetIfEntry2 and remains exclusively borrowed for the call.
        if unsafe { GetIfEntry2(&mut interface) } != ERROR_SUCCESS {
            return false;
        }
        interface.OperStatus == IfOperStatusUp
            && !matches!(
                interface.Type,
                IF_TYPE_TUNNEL | MIB_IF_TYPE_PPP | IF_TYPE_SOFTWARE_LOOPBACK
            )
    }

    fn socket_address_to_ip(address: &SOCKADDR_INET) -> Option<IpAddr> {
        // SAFETY: The active union arm is selected from the family tag written
        // by the Windows IP Helper API in the same structure.
        let family = unsafe { address.si_family };
        if family == AF_INET {
            // SAFETY: `family == AF_INET` establishes the IPv4 union arm.
            let socket = unsafe { address.Ipv4 };
            // SAFETY: S_un_b is a byte-wise view of the initialized IN_ADDR.
            let octets = unsafe { socket.sin_addr.S_un.S_un_b };
            Some(IpAddr::V4(Ipv4Addr::new(
                octets.s_b1,
                octets.s_b2,
                octets.s_b3,
                octets.s_b4,
            )))
        } else if family == AF_INET6 {
            // SAFETY: `family == AF_INET6` establishes the IPv6 union arm.
            let socket = unsafe { address.Ipv6 };
            // SAFETY: Byte is a byte-wise view of the initialized IN6_ADDR.
            let octets = unsafe { socket.sin6_addr.u.Byte };
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        } else {
            None
        }
    }

    fn is_local_candidate(address: IpAddr) -> bool {
        match address {
            IpAddr::V4(address) => {
                address.is_private() || address.is_link_local() || address.is_loopback()
            }
            IpAddr::V6(address) => {
                address.is_unique_local()
                    || address.is_unicast_link_local()
                    || address.is_loopback()
            }
        }
    }

    const fn prefix_is_valid(address: IpAddr, prefix_length: u8) -> bool {
        match address {
            IpAddr::V4(_) => prefix_length <= 32,
            IpAddr::V6(_) => prefix_length <= 128,
        }
    }

    struct OwnedUnicastTable(NonNull<MIB_UNICASTIPADDRESS_TABLE>);

    impl OwnedUnicastTable {
        fn load() -> Result<Self, ManualLanRouteError> {
            let mut table = std::ptr::null_mut();
            // SAFETY: Windows allocates the returned table and documents that it
            // must be released with FreeMibTable. `table` is checked before use.
            let result = unsafe { GetUnicastIpAddressTable(AF_UNSPEC, &mut table) };
            if result != ERROR_SUCCESS {
                return Err(ManualLanRouteError::InspectionFailed);
            }
            NonNull::new(table)
                .map(Self)
                .ok_or(ManualLanRouteError::InspectionFailed)
        }

        fn rows(&self) -> Result<&[MIB_UNICASTIPADDRESS_ROW], ManualLanRouteError> {
            // SAFETY: The pointer is owned by this wrapper and remains valid
            // until Drop. Windows initializes NumEntries and the trailing table.
            let count = usize::try_from(unsafe { self.0.as_ref().NumEntries })
                .map_err(|_| ManualLanRouteError::InspectionFailed)?;
            if count > MAX_UNICAST_ROWS {
                return Err(ManualLanRouteError::InspectionFailed);
            }
            if count == 0 {
                return Ok(&[]);
            }
            // SAFETY: GetUnicastIpAddressTable allocates `NumEntries` contiguous
            // rows beginning at Table[0], bounded above before constructing the
            // slice. The slice cannot outlive `self`.
            let first = unsafe { std::ptr::addr_of!((*self.0.as_ptr()).Table).cast() };
            Ok(unsafe { slice::from_raw_parts(first, count) })
        }
    }

    impl Drop for OwnedUnicastTable {
        fn drop(&mut self) {
            // SAFETY: This pointer was returned by GetUnicastIpAddressTable and
            // is released exactly once by its owning wrapper.
            unsafe { FreeMibTable(self.0.as_ptr().cast()) };
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

    use airwiki_network::ManualLanAddress;

    use super::ManualLanRouteError;

    pub(super) fn validate(_address: &ManualLanAddress) -> Result<(), ManualLanRouteError> {
        // Other platforms retain the existing parser-only behavior in this
        // Windows-scoped hardening change.
        Ok(())
    }

    pub(super) fn active_ipv4_addresses() -> Result<Vec<Ipv4Addr>, ManualLanRouteError> {
        // UDP connect selects the active IPv4 route without sending a packet.
        // Failure simply leaves the advanced fallback unavailable; mDNS and
        // local-only operation continue normally.
        let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
            .map_err(|_| ManualLanRouteError::InspectionFailed)?;
        socket
            .connect(SocketAddr::from((Ipv4Addr::new(192, 0, 2, 1), 9)))
            .map_err(|_| ManualLanRouteError::InspectionFailed)?;
        let address = socket
            .local_addr()
            .map_err(|_| ManualLanRouteError::InspectionFailed)?
            .ip();
        Ok(match address {
            IpAddr::V4(address) if address.is_private() || address.is_link_local() => {
                vec![address]
            }
            IpAddr::V4(_) | IpAddr::V6(_) => Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn ipv4_destination_on_active_prefix_is_accepted() {
        let result = validate_against_subnets(
            IpAddr::V4(Ipv4Addr::new(192, 168, 10, 42)),
            &[ActiveSubnet {
                local_address: IpAddr::V4(Ipv4Addr::new(192, 168, 10, 8)),
                prefix_length: 24,
            }],
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn private_ipv4_destination_on_different_prefix_is_rejected() {
        let result = validate_against_subnets(
            IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40)),
            &[ActiveSubnet {
                local_address: IpAddr::V4(Ipv4Addr::new(192, 168, 10, 8)),
                prefix_length: 24,
            }],
        );

        assert_eq!(result, Err(ManualLanRouteError::NotOnActiveSubnet));
    }

    #[test]
    fn ipv6_ula_destination_on_active_prefix_is_accepted() {
        let result = validate_against_subnets(
            IpAddr::V6("fd42:1234::20".parse::<Ipv6Addr>().unwrap()),
            &[ActiveSubnet {
                local_address: IpAddr::V6("fd42:1234::8".parse::<Ipv6Addr>().unwrap()),
                prefix_length: 64,
            }],
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn ipv6_ula_destination_on_different_prefix_is_rejected() {
        let result = validate_against_subnets(
            IpAddr::V6("fd99::20".parse::<Ipv6Addr>().unwrap()),
            &[ActiveSubnet {
                local_address: IpAddr::V6("fd42:1234::8".parse::<Ipv6Addr>().unwrap()),
                prefix_length: 64,
            }],
        );

        assert_eq!(result, Err(ManualLanRouteError::NotOnActiveSubnet));
    }

    #[test]
    fn bare_ipv6_link_local_destination_is_rejected_as_ambiguous() {
        let result = validate_against_subnets(
            IpAddr::V6("fe80::20".parse::<Ipv6Addr>().unwrap()),
            &[ActiveSubnet {
                local_address: IpAddr::V6("fe80::8".parse::<Ipv6Addr>().unwrap()),
                prefix_length: 64,
            }],
        );

        assert_eq!(result, Err(ManualLanRouteError::ScopedIpv6Required));
    }

    #[test]
    fn loopback_destination_remains_local_without_adapter_enumeration() {
        let result = validate_against_subnets(IpAddr::V4(Ipv4Addr::LOCALHOST), &[]);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn desktop_manual_connection_rejects_ipv6_for_the_mvp() {
        let address = "/ip6/fd42:1234::20/tcp/41000"
            .parse::<ManualLanAddress>()
            .unwrap();

        assert_eq!(
            validate(&address),
            Err(ManualLanRouteError::UnsupportedIpv6)
        );
    }

    #[test]
    fn advertised_fallback_contains_dynamic_port_and_peer_identity() {
        let peer = PeerId::random();
        let listener = "/ip4/0.0.0.0/tcp/61743".parse::<Multiaddr>().unwrap();

        let addresses = build_advertised_addresses(
            &listener,
            peer,
            vec![
                Ipv4Addr::new(192, 168, 1, 25),
                Ipv4Addr::new(192, 168, 1, 25),
                Ipv4Addr::LOCALHOST,
                Ipv4Addr::UNSPECIFIED,
            ],
        )
        .unwrap();

        assert_eq!(
            addresses,
            [format!("/ip4/192.168.1.25/tcp/61743/p2p/{peer}")]
        );
    }
}
