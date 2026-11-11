//! Control messages (`WSACMSGHDR`) for `WSASendMsg`/`WSARecvMsg`: building
//! them, walking them, and the two this crate reads.

use super::*;

/// Builds a one-message control buffer for [`UdpSocket::send_msg`].
pub fn build_cmsg(level: i32, ty: i32, data: &[u8]) -> Vec<u8> {
    let data_off = cmsg_data_offset();
    // cmsg_len counts the unpadded header + payload; the buffer itself is
    // padded so a following message would start aligned.
    let cmsg_len = data_off + data.len();
    let mut buf = vec![0u8; cmsg_align(cmsg_len)];
    let hdr = CMSGHDR {
        cmsg_len,
        cmsg_level: level,
        cmsg_type: ty,
    };
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr() as *mut CMSGHDR, hdr);
    }
    buf[data_off..data_off + data.len()].copy_from_slice(data);
    buf
}

/// Walks a received control buffer, yielding `(level, type, payload)` per
/// message.
///
/// A datagram can carry several — `IP_PKTINFO` alongside `UDP_COALESCED_INFO`,
/// for instance — so a parser that only ever looks at the first one silently
/// misses the rest.
pub fn iter_cmsgs(control: &[u8]) -> impl Iterator<Item = (i32, i32, &[u8])> {
    let data_off = cmsg_data_offset();
    let mut pos = 0usize;
    core::iter::from_fn(move || {
        if pos + data_off > control.len() {
            return None;
        }
        // The buffer comes from the provider with no alignment guarantee of
        // its own beyond the Vec's, so read the header unaligned.
        let hdr: CMSGHDR =
            unsafe { core::ptr::read_unaligned(control[pos..].as_ptr() as *const CMSGHDR) };
        if hdr.cmsg_len < data_off || pos + hdr.cmsg_len > control.len() {
            return None;
        }
        let data = &control[pos + data_off..pos + hdr.cmsg_len];
        pos += cmsg_align(hdr.cmsg_len);
        Some((hdr.cmsg_level, hdr.cmsg_type, data))
    })
}

/// Control message setting the USO segment size for **one** `send_msg`,
/// without touching the socket-wide option.
///
/// ```ignore
/// let ctrl = cmsg_send_segment_size(1200);
/// sock.send_msg(big_buffer, dest, ctrl).await;
/// ```
pub fn cmsg_send_segment_size(segment_size: u16) -> Vec<u8> {
    build_cmsg(
        IPPROTO_UDP.0,
        UDP_SEND_MSG_SIZE,
        &(segment_size as u32).to_ne_bytes(),
    )
}

/// Parses the `UDP_COALESCED_INFO` control message, giving the segment size
/// the stack used when coalescing — i.e. how to cut the received buffer back
/// into datagrams.
///
/// Absent when only one datagram was delivered, which is the common case; a
/// caller should then treat the whole buffer as a single datagram.
///
/// Note that `UDP_COALESCED_INFO` (the control-message type) and
/// `UDP_RECV_MAX_COALESCED_SIZE` (the socket option) are both `3`, in
/// different namespaces — they are not interchangeable.
pub fn parse_udp_coalesced_info(control: &[u8]) -> Option<u32> {
    for (level, ty, data) in iter_cmsgs(control) {
        if level == IPPROTO_UDP.0 && ty == UDP_COALESCED_INFO as i32 && data.len() >= 4 {
            return Some(u32::from_ne_bytes([data[0], data[1], data[2], data[3]]));
        }
    }
    None
}

/// Parses the `IP_PKTINFO` control message out of a `recv_msg` control buffer,
/// returning the local address the datagram arrived on (IPv4).
pub fn parse_ip_pktinfo(control: &[u8]) -> Option<Ipv4Addr> {
    for (level, ty, data) in iter_cmsgs(control) {
        if level == IPPROTO_IP.0 && ty == IP_PKTINFO && data.len() >= size_of::<IN_PKTINFO>() {
            let pktinfo = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const IN_PKTINFO) };
            let addr = unsafe { pktinfo.ipi_addr.S_un.S_addr };
            return Some(Ipv4Addr::from(u32::from_be(addr)));
        }
    }
    None
}
