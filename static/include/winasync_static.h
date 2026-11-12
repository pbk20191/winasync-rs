// winasync's WinRT component as a static library: C declarations.
//
// Link `winasync_static.lib` (cargo build -p winasync-static --release) plus
// the libraries `cargo rustc -p winasync-static -- --print native-static-libs`
// names, and runtimeobject.lib for HSTRING. Nothing here is a reserved name:
// no Dll*, no Ro*. See static/src/lib.rs.
//
// The vtable getters return the class's table for one interface -- the same
// table its objects carry -- as `const void*`, to be cast to the vtable type
// of whichever projection the caller uses (a MIDL `...Vtbl`, C++/WinRT's
// `abi_t<...>`). A method called through it takes the object's interface
// pointer for that interface, exactly as the vtable call would. Under full
// LTO (clang -flto with rustc -Clto=fat -Clinker-plugin-lto, caller target
// features covering rustc's x86-64 baseline) the call inlines.
//
// Generated from the `Export =` lists in src/comio; regenerate rather than
// edit when a class or interface changes.
#pragma once
#include <windows.h>
#include <hstring.h>

#ifdef __cplusplus
extern "C" {
#endif

// The activation factory for a runtime class name, as DllGetActivationFactory
// would hand it out: IActivationFactory, refcount 1, in `factory`.
HRESULT __stdcall winasync_get_activation_factory(HSTRING name, void** factory);

// The class object for a CLSID, as DllGetClassObject would hand it out.
HRESULT __stdcall winasync_get_class_object(const GUID* clsid, const GUID* iid, void** object);


// Pbk20191.WinAsync.Comio.HttpServer (src/comio/http.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_HttpServer_HttpServer_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_HttpServer_IClosable_vtable(void);

// Pbk20191.WinAsync.Comio.HttpRequest (src/comio/http.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_HttpRequest_HttpRequest_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_HttpRequest_IClosable_vtable(void);

// Pbk20191.WinAsync.Comio.HyperVConnector (src/comio/hvsock.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_HyperVConnector_HyperVConnector_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_HyperVConnector_IHyperVConnector_vtable(void);

// Pbk20191.WinAsync.Comio.NamedPipeConnectionListener (src/comio/listener/classes.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_NamedPipeConnectionListener_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_ITransport_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_INamedPipeBind_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_IListenerSecurity_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_INamedPipeListenerSettings_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_IClosable_vtable(void);

// Pbk20191.WinAsync.Comio.UnixConnectionListener (src/comio/listener/classes.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnectionListener_UnixConnectionListener_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnectionListener_ITransport_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnectionListener_IUnixBind_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnectionListener_IListenerSecurity_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnectionListener_IClosable_vtable(void);

// Pbk20191.WinAsync.Comio.HyperVConnectionListener (src/comio/listener/classes.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_HyperVConnectionListener_HyperVConnectionListener_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_HyperVConnectionListener_ITransport_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_HyperVConnectionListener_IHyperVBind_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_HyperVConnectionListener_IClosable_vtable(void);

// Pbk20191.WinAsync.Comio.NamedPipeConnector (src/comio/pipe.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnector_NamedPipeConnector_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_NamedPipeConnector_INamedPipeConnector_vtable(void);

// Pbk20191.WinAsync.Comio.TransportStream (src/comio/stream.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_TransportStream_TransportStream_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_TransportStream_IInputStream_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_TransportStream_IOutputStream_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_TransportStream_IClosable_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_TransportStream_ITransportPeer_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_TransportStream_IMessageStream_vtable(void);

// Pbk20191.WinAsync.Comio.UnixConnector (src/comio/unix.rs)
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnector_UnixConnector_vtable(void);
const void* __stdcall Pbk20191_WinAsync_Comio_UnixConnector_IUnixConnector_vtable(void);

#ifdef __cplusplus
}
#endif
