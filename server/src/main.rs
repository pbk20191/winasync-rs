//! `winasync-server.exe`: the out-of-process server for
//! `Pbk20191.WinAsync.Comio`.
//!
//! COM starts this with `-Embedding` when a client activates one of the six
//! classes out of process. Started by hand it serves just the same, and exits
//! after `IDLE_TIMEOUT` if nobody connects.
//!
//! The Windows subsystem, because COM starts it in the background: a console
//! subsystem would open a console window for every activation. That also
//! means there is nowhere for output to go, so a failure is reported through
//! the exit code alone -- the `HRESULT`, which is what a launcher would want
//! anyway.
#![windows_subsystem = "windows"]

fn main() {
    if let Err(e) = library::comio::server::run() {
        std::process::exit(e.code().0);
    }
}
