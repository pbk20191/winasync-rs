//! The two zero-copy sends of `mswsock`: `TransmitFile` and
//! `TransmitPackets`, over any registered socket.
//!
//! Measured (2026-11-11): both extension pointers are handed out for every
//! family -- TCP, UDP, AF_UNIX, AF_HYPERV -- and are the same functions.
//! `TransmitFile` then works on TCP, AF_UNIX and AF_HYPERV but fails on a
//! UDP socket, connected or not, with `WSAENOTCONN`: it wants a
//! connection-oriented socket. `TransmitPackets` works on all four; on UDP
//! each call is one datagram. A synchronous `TransmitFile` on AF_UNIX did
//! not return until the peer had read -- TCP buffers, AF_UNIX hands over --
//! which overlapped merely makes "pending until consumed".

use super::*;

/// `TransmitFile` on a registered socket: (part of) an open overlapped
/// `file` sent over the socket
/// using the kernel's `TransmitFile` — a zero-copy path straight from the
/// file-system cache, without staging the bytes through user space.
///
/// `offset` is the starting position in the file; `count == 0` sends from
/// `offset` to end of file. Resolves to the number of bytes transmitted.
/// The op completes on this socket's TP_IO like any other overlapped send.
pub(crate) fn transmit_file(
res: &Arc<IoRes>,
    file: &File,
    offset: u64,
    count: u32,
    flags: TransmitFlags,
) -> impl Future<Output = windows_core::Result<usize>> + Send + use<> {
    let sock = res.socket();
    let file_handle = file.handle();
    let tf = flags.to_bits();
    // Keep the file's IoRes (hence its HANDLE) alive for the op's lifetime.
    let keepalive = file.res_arc();
    let fut = unsafe {
        submit(res, keepalive, move |ovl, _keepalive| {
            (*ovl).Anonymous.Anonymous.Offset = offset as u32;
            (*ovl).Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
            let ok = TransmitFile(sock, file_handle, count, 0, Some(ovl), None, tf);
            classify_wsa_overlapped(ok.as_bool())
        })
    };
    async move {
        let (result, _keepalive) = fut.await;
        result
    }
}

/// Sends a sequence of in-memory buffers and/or file ranges as one
/// `TransmitPackets` call — a single-syscall scatter of mixed memory and
/// file sources, transmitted in insertion order. Resolves to the total
/// number of bytes sent. See [`TransmitPackets`] for building the sequence.
pub(crate) fn transmit_packets(
res: &Arc<IoRes>,
    packets: TransmitPackets,
    flags: TransmitFlags,
) -> impl Future<Output = windows_core::Result<usize>> + Send + use<> {
    let sock = res.socket();
    let tf = flags.to_bits();
    // Fetch the extension pointer and issue the op *synchronously*, so the
    // returned async block captures only the (Send) OpFuture — never the
    // non-Send HANDLEs, which are consumed here into the op block.
    let started: windows_core::Result<OpFuture<TransmitPacketsPayload>> = (|| {
        let transmit = get_transmitpackets(sock)?;
        let TransmitPackets { elems } = packets;
        let count = elems.len();
        let payload = TransmitPacketsPayload {
            elems,
            array: Vec::with_capacity(count),
        };
        Ok(unsafe {
            submit(res, payload, move |ovl, p| {
                // Build the element array in place (it and the memory
                // buffers it points at all live in the op block for the
                // op's duration). Pre-reserved, so no realloc moves it.
                p.array.clear();
                for e in p.elems.iter_mut() {
                    let el = match e {
                        Elem::Mem(buf) => TRANSMIT_PACKETS_ELEMENT {
                            dwElFlags: TP_ELEMENT_MEMORY,
                            cLength: buf.len() as u32,
                            Anonymous: TRANSMIT_PACKETS_ELEMENT_0 {
                                // One pointer type for both directions;
                                // a transmit never writes through it.
                                pBuffer: buf.stable_ptr() as *mut c_void,
                            },
                        },
                        Elem::File {
                            handle,
                            offset,
                            length,
                            ..
                        } => TRANSMIT_PACKETS_ELEMENT {
                            dwElFlags: TP_ELEMENT_FILE,
                            cLength: *length,
                            Anonymous: TRANSMIT_PACKETS_ELEMENT_0 {
                                Anonymous: TRANSMIT_PACKETS_ELEMENT_0_0 {
                                    nFileOffset: *offset,
                                    hFile: *handle,
                                },
                            },
                        },
                    };
                    p.array.push(el);
                }
                let ok = transmit(
                    sock,
                    p.array.as_mut_ptr(),
                    p.array.len() as u32,
                    0,
                    ovl,
                    tf,
                );
                classify_wsa_overlapped(ok.as_bool())
            })
        })
    })();
    async move {
        match started {
            Ok(fut) => fut.await.0,
            Err(e) => Err(e),
        }
    }
}

/// One element of a [`TransmitPackets`] sequence: an owned memory buffer, or a
/// range of an open file (kept alive via its `IoRes`).
enum Elem {
    /// Any owned buffer: the kernel only reads it.
    Mem(Box<dyn IoBuf>),
    File {
        /// Keeps the file handle valid for the op's lifetime.
        _keep: Arc<IoRes>,
        handle: HANDLE,
        offset: i64,
        length: u32,
    },
}

/// A builder for `transmit_packets`: an ordered mix of in-memory
/// buffers and open-file ranges, sent in one `TransmitPackets` syscall.
#[derive(Default)]
pub struct TransmitPackets {
    elems: Vec<Elem>,
}

impl TransmitPackets {
    /// A new, empty sequence.
    pub fn new() -> Self {
        Self { elems: Vec::new() }
    }

    /// Appends an in-memory buffer (sent inline): any owned buffer, so a
    /// shared `Arc<[u8]>` header can go out on many connections uncopied.
    pub fn bytes(mut self, buf: impl IoBuf + 'static) -> Self {
        self.elems.push(Elem::Mem(Box::new(buf)));
        self
    }

    /// Appends a byte range of an open overlapped `file` (`length == 0` sends to
    /// end of file). The file is kept alive until the transmit completes.
    pub fn file(mut self, file: &File, offset: i64, length: u32) -> Self {
        self.elems.push(Elem::File {
            _keep: file.res_arc(),
            handle: file.handle(),
            offset,
            length,
        });
        self
    }
}

/// Op-block payload for `TransmitPackets`: owns every memory buffer and file
/// keepalive, plus the element array the kernel reads.
struct TransmitPacketsPayload {
    elems: Vec<Elem>,
    array: Vec<TRANSMIT_PACKETS_ELEMENT>,
}

// All raw handles/pointers held here are plain kernel values / stable heap
// pointers into this same payload; moving the payload is sound.
unsafe impl Send for TransmitPacketsPayload {}
