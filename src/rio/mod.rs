//! Registered I/O (RIO): a second completion engine, beside the overlapped /
//! TP_IO core, for socket workloads where per-operation overhead dominates.
//!
//! # Why this is a separate engine and not a transport
//!
//! Everything in [`crate::tp::io`] and [`crate::comio`] is built on
//! [`crate::common::overlapped`]: one `OVERLAPPED`-headed op block per
//! operation, handed to the kernel, completed by a TP_IO callback. RIO shares
//! none of that machinery:
//!
//! * **Buffers are registered once, not handed over per op.**
//!   `RIORegisterBuffer` page-locks a slab and returns a `RIO_BUFFERID`;
//!   each operation then names a `RIO_BUF { BufferId, Offset, Length }`
//!   instead of a pointer the kernel must probe and lock again. This is where
//!   most of RIO's advantage comes from, and it is why RIO cannot wear the
//!   `read(buf: Vec<u8>) -> (Result<usize>, Vec<u8>)` shape the other
//!   transports use: a buffer has to live in the registered slab, so the API
//!   hands out slices of one rather than taking an owned `Vec`.
//!
//! * **Completions are drained, not delivered.** There is no per-op
//!   `OVERLAPPED` and no TP_IO callback. Results accumulate in a completion
//!   queue that the application empties with `RIODequeueCompletion`, in
//!   batches.
//!
//! * **Request queues are sized up front.** `RIOCreateRequestQueue` fixes the
//!   maximum outstanding sends and receives per socket, so backpressure is
//!   explicit rather than unbounded.
//!
//! So RIO lands here as its own module behind its own `rio` feature, leaving
//! the overlapped transports untouched. It is opt-in in both senses: you
//! enable the feature, and you write against its buffer model deliberately.
//!
//! # Scope
//!
//! RIO covers send and receive only — there is no RIO accept or connect, so
//! connection setup still goes through `AcceptEx`/`ConnectEx`. It is a TCP/IP
//! provider facility: `WSA_FLAG_REGISTERED_IO` is rejected with `WSAEINVAL`
//! at socket creation for AF_UNIX, AF_HYPERV and raw sockets, so the
//! non-IP transports in `tp` cannot use it.
//!
//! # Experimental
//!
//! This engine exists to cover a *concept* — registered buffers plus a drained
//! completion queue — not because it is faster here. Published measurements
//! put RIO ahead only at high datagram rates on ≥10 GbE links with many
//! concurrent sockets; at 1 GbE the difference is nil, and on loopback at low
//! concurrency IOCP has measured an order of magnitude *better*, because IOCP
//! can complete an operation inline (see
//! `IoReg::set_skip_completion_on_success`) while RIO must round-trip every
//! result through the queue. Every test here runs on loopback — RIO's worst
//! case — so this crate's suite can show that the engine is correct, never
//! that it is fast.
//!
//! Two further constraints shape the API and are not going to change:
//! operations cannot be cancelled (only `closesocket`, whose completions
//! still arrive afterwards), and neither queue is internally synchronized.
// Engine under construction: pieces land before their consumers do, as in the
// other runtime modules.
#![allow(dead_code)]

pub mod buffer;
pub mod cq;
pub mod rq;
pub mod socket;
pub mod tcp;
pub(crate) mod table;

#[allow(unused_imports)]
pub use buffer::{RioBufferPool, RioSlice};
#[allow(unused_imports)]
pub use cq::RioCq;
#[allow(unused_imports)]
pub use rq::RqConfig;
#[allow(unused_imports)]
pub use socket::RioUdpSocket;
#[allow(unused_imports)]
pub use tcp::{RioTcpListener, RioTcpStream};
