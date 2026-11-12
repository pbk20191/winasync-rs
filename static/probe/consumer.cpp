// A C++ program linking winasync_static.lib the way a static consumer would:
// an object comes from the activation entry point, and a method is reached
// two ways -- through the object's own vtable, as any COM caller writes it,
// and through the class's exported table. build.sh links this with full
// cross-language LTO and reports what the inliner did with each.
//
// The class is NamedPipeConnectionListener and the method IClosable::Close,
// which an unbound listener answers at once.
#include "../include/winasync_static.h"
#include <winstring.h>
#include <cstdio>

// COM, by hand: the layouts of the three interfaces touched, so that no
// projection stands between this file and the ABI.
struct IUnknownVtbl_ {
    HRESULT (__stdcall *QueryInterface)(void*, const GUID*, void**);
    ULONG (__stdcall *AddRef)(void*);
    ULONG (__stdcall *Release)(void*);
};
struct IInspectableVtbl_ : IUnknownVtbl_ {
    HRESULT (__stdcall *GetIids)(void*, ULONG*, GUID**);
    HRESULT (__stdcall *GetRuntimeClassName)(void*, HSTRING*);
    HRESULT (__stdcall *GetTrustLevel)(void*, int*);
};
struct IActivationFactoryVtbl_ : IInspectableVtbl_ {
    HRESULT (__stdcall *ActivateInstance)(void*, void**);
};
struct IClosableVtbl_ : IInspectableVtbl_ {
    HRESULT (__stdcall *Close)(void*);
};
template <class V> struct Object { const V* vtbl; };

static const GUID IID_IActivationFactory_ = {0x00000035, 0x0000, 0x0000, {0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46}};
static const GUID IID_IClosable_ = {0x30D5A829, 0x7FA4, 0x4026, {0x83, 0xBB, 0xD7, 0x5B, 0xAE, 0x4E, 0xA9, 0x9E}};

// Through the object's vtable: what a projection generates.
__attribute__((noinline)) HRESULT close_via_object(Object<IClosableVtbl_>* p) {
    return p->vtbl->Close(p);
}

// Through the class's table: the same slot, from a constant.
__attribute__((noinline)) HRESULT close_via_class_table(Object<IClosableVtbl_>* p) {
    auto* table = static_cast<const IClosableVtbl_*>(
        Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_IClosable_vtable());
    return table->Close(p);
}

static Object<IClosableVtbl_>* make_listener() {
    HSTRING name = nullptr;
    const wchar_t text[] = L"Pbk20191.WinAsync.Comio.NamedPipeConnectionListener";
    if (FAILED(WindowsCreateString(text, sizeof(text) / sizeof(wchar_t) - 1, &name))) return nullptr;
    void* factory = nullptr;
    HRESULT hr = winasync_get_activation_factory(name, &factory);
    WindowsDeleteString(name);
    if (FAILED(hr)) { std::printf("activation factory: 0x%08lx\n", hr); return nullptr; }
    auto* f = static_cast<Object<IActivationFactoryVtbl_>*>(factory);
    void* inspectable = nullptr;
    hr = f->vtbl->ActivateInstance(f, &inspectable);
    f->vtbl->Release(f);
    if (FAILED(hr)) { std::printf("ActivateInstance: 0x%08lx\n", hr); return nullptr; }
    auto* i = static_cast<Object<IInspectableVtbl_>*>(inspectable);
    void* closable = nullptr;
    hr = i->vtbl->QueryInterface(i, &IID_IClosable_, &closable);
    i->vtbl->Release(i);
    if (FAILED(hr)) { std::printf("QueryInterface(IClosable): 0x%08lx\n", hr); return nullptr; }
    return static_cast<Object<IClosableVtbl_>*>(closable);
}

int main() {
    auto* a = make_listener();
    auto* b = make_listener();
    if (!a || !b) return 2;
    HRESULT ha = close_via_object(a);
    HRESULT hb = close_via_class_table(b);
    // The exported table is the object's: same slot, same function.
    bool same = reinterpret_cast<const void*>(a->vtbl->Close) ==
        reinterpret_cast<const void*>(static_cast<const IClosableVtbl_*>(
            Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_IClosable_vtable())->Close);
    a->vtbl->Release(a);
    b->vtbl->Release(b);
    std::printf("close via object 0x%08lx, via class table 0x%08lx, same slot %s\n", ha, hb, same ? "yes" : "no");
    return SUCCEEDED(ha) && SUCCEEDED(hb) && same ? 0 : 1;
}
