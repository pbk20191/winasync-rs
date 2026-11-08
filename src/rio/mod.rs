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
//! completion queue — rather than because it is always faster. What it
//! actually does here is measured, not assumed: `bench.rs` compares it with
//! the overlapped `UdpSocket` across queue depths, buffers recycled on both
//! sides and sends posted before any are awaited, so neither engine is
//! serialized against the other.
//!
//! On this machine, 64-byte datagrams over loopback: the overlapped path
//! leads slightly at **queue depth 1** (~0.94x), parity arrives at **depth
//! 2**, and RIO pulls to ~**1.5x** by depth 128. Deferred sends plus one
//! commit are doing much of that — RIO can hand a whole batch to the provider
//! in one crossing, and the overlapped API has no equivalent.
//!
//! Treat those as this workload's numbers only. Published comparisons of
//! *request/response* traffic at depth 1 put RIO far behind IOCP, because
//! IOCP can complete an operation inline (see
//! `IoReg::set_skip_completion_on_success`) while RIO must round-trip every
//! result through the queue — and a loopback suite cannot say anything about
//! the ≥10 GbE links where RIO's published wins come from. Run the benchmark
//! on the traffic you actually have.
//!
//! Two further constraints shape the API and are not going to change:
//! operations cannot be cancelled (only `closesocket`, whose completions
//! still arrive afterwards), and neither queue is internally synchronized.
// Engine under construction: pieces land before their consumers do, as in the
// other runtime modules.
#![allow(dead_code)]

mod bench;
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
