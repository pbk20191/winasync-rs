//! COM components exposing named pipe, AF_UNIX and Hyper-V socket transports
//! through the same WinRT stream interfaces (`IInputStream`/`IOutputStream`/
//! `IAsyncOperationWithProgress`) as `StreamSocket`, backed by Win32 overlapped
//! IO on the default thread-pool callback environment (`pcbe = NULL`).
//!
//! All three are transports WinRT itself does not expose: `StreamSocket` has no
//! named-pipe, AF_UNIX or AF_HYPERV support, and `HostName` cannot even name
//! those endpoints.
//!
//! # Where the interfaces come from
//!
//! Not from this module. `metadata/winasync.rdl` describes the whole
//! WinRT surface, `tools/winmdgen` compiles it to the `.winmd` this component
//! publishes and runs `windows-bindgen` over that, and the result is
//! [`bindings`]. Everything here implements those generated traits.
//!
//! That inversion is forced, not stylistic. A hand-written `#[interface]`
//! cannot derive from `IInspectable`: `windows-interface` emits
//! `Parent_Vtbl::new::<Identity, OFFSET>` and needs a `Parent_Impl` trait,
//! and `IInspectable` has neither — its `new` takes three type parameters and
//! `IInspectable_Impl` does not exist. So a hand-written interface always
//! starts three vtable slots in, where a projection expects six. Only bindgen
//! emits the latter, which means metadata has to come first.
//!
//! # What `IInspectable` answers here, and what it does not
//!
//! * `GetRuntimeClassName` reports the class's name. The hook is narrow and
//!   undocumented: the method returns `RuntimeName::NAME` of the **first**
//!   type named in `#[implement]`, so each object names its generated *class*
//!   type there rather than an interface. [`activation::CLASSES`] keys its
//!   WinRT activation off those same constants, so an object has one name
//!   whichever world asks.
//! * `GetIids` reports an empty set. `IInspectable_Vtbl::new` installs a free
//!   function that writes `count = 0`, `values = null` and returns `S_OK`.
//!   Unlike the name it takes no type parameter at all, so there is no point
//!   at which an object's real interface chains could reach it — short of
//!   hand-writing the identity vtable that `#[implement]` generates. A
//!   consumer doing type discovery is told "no interfaces" rather than "I
//!   cannot say", which is a confident wrong answer; no projection asks, so
//!   it is not on the critical path.
//! * `GetTrustLevel` does reach our code, through `IUnknownImpl`, but the
//!   macro hardcodes what it returns to `0`, `BaseTrust`. The attribute does
//!   accept `TrustLevel = Partial | Full`; `BaseTrust` is what these objects
//!   are, so nothing passes it.
//!
//! `tests::iinspectable_is_shaped_but_hollow` pins all three answers, so a
//! `windows-rs` upgrade that changes any of them shows up as a failure rather
//! than a silent shift.
//!
//! # What is no longer here
//!
//! Until 2026-11-09 this module also carried an automation layer: `IDispatch`
//! on every class, a generated `.tlb`, and connection points delivering
//! `ConnectionReceived` to `IDispatch` sinks. It worked, and it was measured
//! working from PowerShell, VBScript and WSH. It is gone.
//!
//! It could only ever describe a *blocking translation* of this surface.
//! Automation has no parameterized interfaces, so `IAsyncOperation<T>` could
//! not appear in a type library at all; a script got `Accept(timeoutMs)` and a
//! queue where the vtable has an awaitable. Carrying a second, differently
//! shaped description of every member — and the drift risk that comes with it
//! — is only worth it for hosts that can never consume WinRT. Those hosts are
//! out of scope (see `docs/comio-com-winrt-status.md`), so the layer had no
//! audience left. Scripts come back through the `.winmd` and a projection,
//! with the async model intact rather than translated away.
#![allow(dead_code)]

pub mod activation;
pub mod asyncop;
pub mod bindings;
pub mod buffer;
pub mod connector;
pub mod future_op;
#[cfg(feature = "http")]
pub mod http;
pub mod hvsock;
pub mod listener;
pub mod pipe;
pub(crate) mod pool;
pub mod settled;
pub mod stream;
pub mod tpio;
pub mod unix;
pub mod unload;
pub mod waitop;

// Public transport surface. Nothing outside `comio`'s own tests consumes
// these yet (this binary has no other caller wired up), hence
// `#[allow(unused_imports)]` alongside the module's existing
// `#![allow(dead_code)]` — both are re-export/API-shape warnings, not bugs.
#[allow(unused_imports)]
pub use pipe::{NamedPipeConnector, NamedPipeListener, NamedPipeOptions};
// The WinRT-native, `StreamSocketListener`-shaped surface (`listener.rs`):
// one shared interface, one concrete type per transport, so each exposes only
// the bind its addressing supports.
#[allow(unused_imports)]
pub use listener::{
    HyperVConnectionListener, NamedPipeConnectionListener, UnixConnectionListener,
};
// The listener-side interfaces now come from metadata; see [`bindings`].
#[allow(unused_imports)]
pub use bindings::Pbk20191::WinAsync::Comio::{
    IConnectionListener, IConnectionReceivedArgs, IHyperVBind, INamedPipeBind,
    IUnixBind,
};
// The client half's interfaces now come from metadata; see [`bindings`].
#[allow(unused_imports)]
pub use bindings::Pbk20191::WinAsync::Comio::{
    IHyperVConnector, INamedPipeConnector, ITransport, IUnixConnector,
};
// `stream::from_reg` takes/returns crate-private types (`Arc<tpio::IoReg>`),
// so it stays `pub(crate)` here rather than `pub` (the latter would be a hard
// "private type in public interface" error, E0446) — this re-export just
// gives it a shorter crate-internal path (`comio::from_reg` instead of
// `comio::stream::from_reg`) for `pipe`/`unix`'s own use.
#[allow(unused_imports)]
pub(crate) use stream::from_reg;
#[allow(unused_imports)]
pub use unix::{UnixConnector, UnixStreamListener};
#[allow(unused_imports)]
pub use hvsock::{HvStreamListener, HyperVConnector};

pub(crate) use crate::common::check::check;

#[cfg(test)]
mod tests;
#[cfg(all(test, feature = "tp"))]
mod bench;
