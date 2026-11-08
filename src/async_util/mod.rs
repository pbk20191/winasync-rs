//! Runtime-neutral async primitives shared across the `tp` / `dq` runtimes (and
//! any hand-rolled thread loops). Everything here depends only on `std` +
//! `core::task` — it knows nothing about a specific executor, so it never
//! couples the runtimes to each other; it is just the glue they compose through.

pub mod oneshot;
