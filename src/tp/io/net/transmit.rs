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
//!
//! # What a `TransmitPackets` sends
//!
//! A [`TransmitSequence`]: any owned buffer, a [`FileRange`], or those
//! joined with [`then`](TransmitSequence::then). The whole sequence moves
//! into the operation and comes back out of it, so a header shared as an
//! `Arc<[u8]>` goes out on many connections uncopied, and no element is
//! boxed or erased: the sequence's type says what it holds.
//!
//! ```ignore
//! stream.transmit_packets(header.then(body).then(FileRange::new(&file, 0, 0)), flags)
//! ```
//!
//! A sequence of one type but any length is a [`List`]; one of mixed kinds
//! and any length is a `List<RawBuf>`, with [`RawBuf::of`] over each.

use super::*;

/// `TransmitFile` on a registered socket: (part of) an open overlapped
/// `file` sent over the socket using the kernel's `TransmitFile` -- a
/// zero-copy path straight from the file-system cache, without staging the
/// bytes through user space.
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

/// Sends `packets` as one `TransmitPackets` call -- a single-syscall scatter
/// of memory and file sources, in order. Resolves to the total number of
/// bytes sent.
pub(crate) fn transmit_packets<S: TransmitSequence>(
    res: &Arc<IoRes>,
    packets: S,
    flags: TransmitFlags,
) -> impl Future<Output = windows_core::Result<usize>> + Send + use<S> {
    let sock = res.socket();
    let tf = flags.to_bits();
    // Fetch the extension pointer and issue the op *synchronously*, so the
    // returned async block captures only the (Send) OpFuture.
    let started: windows_core::Result<OpFuture<Payload<S>>> = (|| {
        let transmit = get_transmitpackets(sock)?;
        let count = packets.count();
        let payload = Payload {
            seq: packets,
            array: Vec::with_capacity(count),
        };
        Ok(unsafe {
            submit(res, payload, move |ovl, p| {
                // The element array is built in place: it and everything it
                // points at live in the op block for the op's duration.
                p.array.clear();
                p.seq.fill(&mut p.array);
                let ok = transmit(sock, p.array.as_mut_ptr(), p.array.len() as u32, 0, ovl, tf);
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

/// Op-block payload: the sequence, and the element array the kernel reads.
struct Payload<S> {
    seq: S,
    array: Vec<TRANSMIT_PACKETS_ELEMENT>,
}

// The array's pointers point into `seq`, which is `Send`; the array is not
// shared.
unsafe impl<S: Send> Send for Payload<S> {}

/// What `transmit_packets` sends: one or more memory buffers and file
/// ranges, in order, owned by the operation while it runs.
///
/// Every [`IoBuf`] is one element; a [`FileRange`] is one; `a.then(b)` is
/// `a`'s elements followed by `b`'s; a [`List`] is each of its items in
/// turn. The elements must point at memory that stays where it is while the
/// value exists -- which is [`IoBuf`]'s own contract.
pub trait TransmitSequence: Send + 'static {
    /// How many elements [`fill`](Self::fill) appends.
    fn count(&self) -> usize;

    /// Appends this sequence's elements, pointing into `self`.
    fn fill(&self, out: &mut Vec<TRANSMIT_PACKETS_ELEMENT>);

    /// This sequence, then `next`.
    fn then<B: TransmitSequence>(self, next: B) -> Then<Self, B>
    where
        Self: Sized,
    {
        Then(self, next)
    }
}

impl<B: IoBuf + 'static> TransmitSequence for B {
    fn count(&self) -> usize {
        1
    }
    fn fill(&self, out: &mut Vec<TRANSMIT_PACKETS_ELEMENT>) {
        out.push(TRANSMIT_PACKETS_ELEMENT {
            dwElFlags: TP_ELEMENT_MEMORY,
            cLength: IoBuf::len(self) as u32,
            Anonymous: TRANSMIT_PACKETS_ELEMENT_0 {
                // One pointer type for both directions; a transmit never
                // writes through it.
                pBuffer: self.stable_ptr() as *mut c_void,
            },
        });
    }
}

/// A byte range of an open overlapped file, kept open until the send
/// completes.
pub struct FileRange {
    /// Keeps the file handle valid for the op's lifetime.
    _keep: Arc<IoRes>,
    handle: HANDLE,
    offset: i64,
    length: u32,
}

// A handle value and the registration that keeps it open.
unsafe impl Send for FileRange {}

impl FileRange {
    /// `length` bytes of `file` from `offset`; `length == 0` sends to the
    /// end of the file.
    pub fn new(file: &File, offset: i64, length: u32) -> FileRange {
        FileRange {
            _keep: file.res_arc(),
            handle: file.handle(),
            offset,
            length,
        }
    }
}

impl TransmitSequence for FileRange {
    fn count(&self) -> usize {
        1
    }
    fn fill(&self, out: &mut Vec<TRANSMIT_PACKETS_ELEMENT>) {
        out.push(TRANSMIT_PACKETS_ELEMENT {
            dwElFlags: TP_ELEMENT_FILE,
            cLength: self.length,
            Anonymous: TRANSMIT_PACKETS_ELEMENT_0 {
                Anonymous: TRANSMIT_PACKETS_ELEMENT_0_0 {
                    nFileOffset: self.offset,
                    hFile: self.handle,
                },
            },
        });
    }
}

/// Two sequences, one after the other: what [`TransmitSequence::then`]
/// builds.
pub struct Then<A, B>(pub A, pub B);

impl<A: TransmitSequence, B: TransmitSequence> TransmitSequence for Then<A, B> {
    fn count(&self) -> usize {
        self.0.count() + self.1.count()
    }
    fn fill(&self, out: &mut Vec<TRANSMIT_PACKETS_ELEMENT>) {
        self.0.fill(out);
        self.1.fill(out);
    }
}

/// Any number of sequences of one type, in order.
pub struct List<T>(pub Vec<T>);

impl<T: TransmitSequence> TransmitSequence for List<T> {
    fn count(&self) -> usize {
        self.0.iter().map(TransmitSequence::count).sum()
    }
    fn fill(&self, out: &mut Vec<TRANSMIT_PACKETS_ELEMENT>) {
        for item in &self.0 {
            item.fill(out);
        }
    }
}
