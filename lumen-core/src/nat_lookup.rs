#![cfg(target_os = "macos")]
//! Original destination recovery for pf-redirected connections on macOS.
//!
//! When macOS pf's `rdr` rule rewrites a packet's destination, the original
//! target is stored in pf's NAT state table. This module queries that table
//! using the `DIOCNATLOOK` ioctl on `/dev/pf`.
//!
//! Struct layouts verified against macOS 14 xnu `net/pfvar.h` headers.
//! `pfioc_natlook` is 84 bytes on this platform (xport is a 4-byte union,
//! not u16 as some implementations incorrectly assume).

use anyhow::{Context, Result};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use tokio::net::TcpStream;

/// 128-bit address union matching `struct pf_addr` from `net/pfvar.h`.
/// The union contains overlapping views: IPv4 (4 bytes), IPv6 (16 bytes),
/// u8[16], u16[8], u32[4]. Total size: 16 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct PfAddr {
    addr: [u8; 16],
}

impl Default for PfAddr {
    fn default() -> Self {
        Self { addr: [0u8; 16] }
    }
}

/// Port/SPI union matching `union pf_state_xport` from `net/pfvar.h`.
/// Contains overlapping: u16 port, u16 call_id, u32 spi.
/// Total size: 4 bytes (aligned to u32).
#[repr(C)]
#[derive(Clone, Copy)]
union PfStateXport {
    port: u16,
    _call_id: u16,
    _spi: u32,
}

impl Default for PfStateXport {
    fn default() -> Self {
        Self { _spi: 0 }
    }
}

/// Matches `struct pfioc_natlook` from macOS `net/pfvar.h`.
/// Verified: total size = 84 bytes on macOS 14 (Sonoma).
///
/// Field offsets (verified with offsetof()):
///   saddr    @  0  (16 bytes)
///   daddr    @ 16  (16 bytes)
///   rsaddr   @ 32  (16 bytes)
///   rdaddr   @ 48  (16 bytes)
///   sxport   @ 64  ( 4 bytes)
///   dxport   @ 68  ( 4 bytes)
///   rsxport  @ 72  ( 4 bytes)
///   rdxport  @ 76  ( 4 bytes)
///   af       @ 80  ( 1 byte, sa_family_t)
///   proto    @ 81  ( 1 byte)
///   proto_variant @ 82 ( 1 byte)
///   direction @ 83 ( 1 byte)
#[repr(C)]
#[derive(Clone, Copy)]
struct PfiocNatlook {
    saddr: PfAddr,
    daddr: PfAddr,
    rsaddr: PfAddr,
    rdaddr: PfAddr,
    sxport: PfStateXport,
    dxport: PfStateXport,
    rsxport: PfStateXport,
    rdxport: PfStateXport,
    af: u8,
    proto: u8,
    proto_variant: u8,
    direction: u8,
}

impl Default for PfiocNatlook {
    fn default() -> Self {
        // Safety: all-zeros is valid for this packed C struct
        unsafe { std::mem::zeroed() }
    }
}

/// DIOCNATLOOK ioctl number for macOS pf.
///
/// From pfvar.h: `#define DIOCNATLOOK _IOWR('D', 23, struct pfioc_natlook)`
///
/// macOS ioctl encoding:
///   IOC_INOUT (0xC0000000) | (sizeof(pfioc_natlook) << 16) | ('D' << 8) | 23
///
/// Verified: 0xc0544417 (size=84, group='D'=0x44, number=23)
const DIOCNATLOOK: libc::c_ulong = {
    let size = std::mem::size_of::<PfiocNatlook>() as libc::c_ulong;
    let ioc_inout: libc::c_ulong = 0xC000_0000;
    ioc_inout | (size << 16) | (0x44 << 8) | 23
};

const AF_INET: u8 = libc::AF_INET as u8;
const IPPROTO_TCP: u8 = libc::IPPROTO_TCP as u8;
const PF_OUT: u8 = 1;

/// Handle to `/dev/pf` for NAT state lookups. Requires root.
pub struct NatHandle {
    fd: RawFd,
}

// The fd is only used for read-only ioctl queries; safe across threads.
unsafe impl Send for NatHandle {}
unsafe impl Sync for NatHandle {}

impl NatHandle {
    /// Open `/dev/pf` in read-only mode. Requires root privileges.
    pub fn open() -> Result<Arc<Self>> {
        let fd = unsafe { libc::open(c"/dev/pf".as_ptr(), libc::O_RDONLY) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error())
                .context("Failed to open /dev/pf (requires root privileges)");
        }
        Ok(Arc::new(Self { fd }))
    }
}

impl Drop for NatHandle {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

impl AsRawFd for NatHandle {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

/// Query pf's NAT state table for the original destination of a redirected
/// connection.
///
/// `client_addr`: source address of the incoming connection (the real client)
/// `local_addr`: the address the connection arrived on (our listen addr after rdr)
fn lookup_original_dest(
    pf: &NatHandle,
    client_addr: SocketAddrV4,
    local_addr: SocketAddrV4,
) -> Result<SocketAddrV4> {
    let mut nl = PfiocNatlook {
        af: AF_INET,
        proto: IPPROTO_TCP,
        direction: PF_OUT,
        ..PfiocNatlook::default()
    };

    // Source: the connecting client
    nl.saddr.addr[..4].copy_from_slice(&client_addr.ip().octets());
    nl.sxport = PfStateXport {
        port: client_addr.port().to_be(),
    };

    // Destination: what we see (our proxy listen address, post-redirect)
    nl.daddr.addr[..4].copy_from_slice(&local_addr.ip().octets());
    nl.dxport = PfStateXport {
        port: local_addr.port().to_be(),
    };

    // Try PF_OUT first (standard for rdr rules)
    let ret = unsafe { libc::ioctl(pf.as_raw_fd(), DIOCNATLOOK, &mut nl as *mut PfiocNatlook) };

    if ret < 0 {
        // Fallback: try PF_IN direction
        nl.direction = 0; // PF_IN
        let ret2 =
            unsafe { libc::ioctl(pf.as_raw_fd(), DIOCNATLOOK, &mut nl as *mut PfiocNatlook) };
        if ret2 < 0 {
            return Err(std::io::Error::last_os_error())
                .context("DIOCNATLOOK failed (connection may not have been redirected by pf)");
        }
    }

    let mut ip_bytes = [0u8; 4];
    ip_bytes.copy_from_slice(&nl.rdaddr.addr[..4]);
    let ip = Ipv4Addr::from(ip_bytes);
    let port = u16::from_be(unsafe { nl.rdxport.port });

    Ok(SocketAddrV4::new(ip, port))
}

/// Determine the original destination for a pf-redirected TCP connection.
///
/// Tries DIOCNATLOOK first. Falls back to checking if `getsockname()` returned
/// a different address than the listen address (works with `divert-to` rules).
///
/// Returns `Err` if the original destination cannot be determined.
pub fn get_original_dest(
    pf: &NatHandle,
    _stream: &TcpStream,
    client_addr: SocketAddr,
    local_addr: SocketAddr,
    listen_addr: SocketAddr,
) -> Result<SocketAddrV4> {
    let client_v4 = match client_addr {
        SocketAddr::V4(a) => a,
        _ => anyhow::bail!("IPv6 not yet supported for NAT lookup"),
    };
    let local_v4 = match local_addr {
        SocketAddr::V4(a) => a,
        _ => anyhow::bail!("IPv6 not yet supported for NAT lookup"),
    };

    match lookup_original_dest(pf, client_v4, local_v4) {
        Ok(dest) => {
            let dest_sa = SocketAddr::V4(dest);
            if dest_sa == listen_addr {
                anyhow::bail!(
                    "Loop detected: original dest {} equals listen addr {}",
                    dest_sa,
                    listen_addr
                );
            }
            Ok(dest)
        }
        Err(e) => {
            tracing::debug!("DIOCNATLOOK failed: {:#}, trying getsockname fallback", e);

            // Fallback: if the local address differs from our listen address,
            // it may be the original destination (works with divert-to rules)
            if local_addr != listen_addr && local_v4.port() != listen_addr.port() {
                Ok(local_v4)
            } else {
                Err(e).context("Could not determine original destination")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_kernel() {
        assert_eq!(std::mem::size_of::<PfAddr>(), 16);
        assert_eq!(std::mem::size_of::<PfStateXport>(), 4);
        assert_eq!(std::mem::size_of::<PfiocNatlook>(), 84);
    }

    #[test]
    fn struct_offsets_match_kernel() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(PfiocNatlook, saddr), 0);
        assert_eq!(offset_of!(PfiocNatlook, daddr), 16);
        assert_eq!(offset_of!(PfiocNatlook, rsaddr), 32);
        assert_eq!(offset_of!(PfiocNatlook, rdaddr), 48);
        assert_eq!(offset_of!(PfiocNatlook, sxport), 64);
        assert_eq!(offset_of!(PfiocNatlook, dxport), 68);
        assert_eq!(offset_of!(PfiocNatlook, rsxport), 72);
        assert_eq!(offset_of!(PfiocNatlook, rdxport), 76);
        assert_eq!(offset_of!(PfiocNatlook, af), 80);
        assert_eq!(offset_of!(PfiocNatlook, proto), 81);
        assert_eq!(offset_of!(PfiocNatlook, proto_variant), 82);
        assert_eq!(offset_of!(PfiocNatlook, direction), 83);
    }

    #[test]
    fn diocnatlook_ioctl_number() {
        assert_eq!(DIOCNATLOOK, 0xc054_4417);
    }
}
