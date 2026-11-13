//! `static/include/winasync_static.h`: the C and C++ declarations of the
//! static library's exported vtable getters, typed.
//!
//! The vtable layouts come from `src/comio/bindings.rs`, which `bindgen`
//! derived from the `.winmd`: every `*_Vtbl` struct there is the interface's
//! ABI with each parameter already reduced to what crosses it (`void*` for
//! an interface or a string, the scalar for a scalar), so the C struct is a
//! transcription, not a second derivation. The three `Windows.*` interfaces
//! the classes implement are spelled out here by hand -- their ABI is the
//! SDK's and does not change -- and `IInspectable` is written once as the
//! base every WinRT vtable starts with.
//!
//! Which getters exist comes from the `#[implement(..., Export = ...)]`
//! lists in `src/comio`, read the same way the macro reads them. A runtime
//! class in a list stands for its default interface; the macro names that
//! getter by the bare prefix because it cannot see the interface, and this
//! generator resolves it from the `Interface for <Class>` impl bindgen
//! emits, so the header says which interface the table is for.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

const NAMESPACE: &str = "Pbk20191.WinAsync.Comio";
const PREFIX: &str = "Pbk20191_WinAsync_Comio_";
const OUT: &str = "static/include/winasync_static.h";

/// One slot: the method name and its C parameter types, `this` first.
struct Slot {
    name: String,
    params: Vec<String>,
}

/// An interface whose table a getter returns.
struct Interface {
    /// The C name of its vtable struct.
    c_vtbl: String,
    /// The C++/WinRT projected type, for the overloads that take it.
    winrt: String,
    /// The C++/WinRT header's include guard, which gates those overloads.
    winrt_guard: &'static str,
    slots: Vec<Slot>,
}

/// One exported getter: the symbol, and the interface whose table it returns.
struct Getter {
    symbol: String,
    interface: String,
}

struct Class {
    name: String,
    source: String,
    getters: Vec<Getter>,
}

pub fn generate(root: &Path) {
    let bindings = root.join("src/comio/bindings.rs");
    let text = std::fs::read_to_string(&bindings).unwrap_or_else(|e| panic!("{}: {e}", bindings.display()));
    let file = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", bindings.display()));

    let mut interfaces = windows_interfaces();
    let mut default_interface = BTreeMap::new();
    collect_bindings(&file.items, &mut interfaces, &mut default_interface);

    let mut classes = Vec::new();
    collect_classes(&root.join("src/comio"), root, &default_interface, &mut classes);
    classes.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(!classes.is_empty(), "no `Export = {PREFIX}...` lists found under src/comio");

    // Only the interfaces some class exports a table for.
    let used: BTreeMap<&str, &Interface> = classes
        .iter()
        .flat_map(|c| c.getters.iter().map(|g| g.interface.as_str()))
        .map(|name| {
            let i = interfaces
                .get(name)
                .unwrap_or_else(|| panic!("interface `{name}` has no vtable in bindings.rs and is not a known Windows one"));
            (name, i)
        })
        .collect();

    let out = root.join(OUT);
    std::fs::write(&out, render(&classes, &used)).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!(
        "wrote {} ({} classes, {} getters, {} interfaces)",
        out.display(),
        classes.len(),
        classes.iter().map(|c| c.getters.len()).sum::<usize>(),
        used.len()
    );
}

/// `IClosable`, `IInputStream`, `IOutputStream`: the `Windows.*` interfaces
/// the classes implement, whose vtables live in the `windows` crate rather
/// than in our bindings.
fn windows_interfaces() -> BTreeMap<String, Interface> {
    let slot = |name: &str, params: &[&str]| Slot {
        name: name.to_owned(),
        params: params.iter().map(|p| (*p).to_owned()).collect(),
    };
    let mut m = BTreeMap::new();
    m.insert(
        "IClosable".to_owned(),
        Interface {
            c_vtbl: "winasync_Windows_Foundation_IClosableVtbl".to_owned(),
            winrt: "winrt::Windows::Foundation::IClosable".to_owned(),
            winrt_guard: "WINRT_Windows_Foundation_H",
            slots: vec![slot("Close", &["void*"])],
        },
    );
    m.insert(
        "IInputStream".to_owned(),
        Interface {
            c_vtbl: "winasync_Windows_Storage_Streams_IInputStreamVtbl".to_owned(),
            winrt: "winrt::Windows::Storage::Streams::IInputStream".to_owned(),
            winrt_guard: "WINRT_Windows_Storage_Streams_H",
            // ReadAsync(IBuffer* buffer, UINT32 count, InputStreamOptions options, IAsyncOperationWithProgress** result)
            slots: vec![slot("ReadAsync", &["void*", "void*", "UINT32", "UINT32", "void**"])],
        },
    );
    m.insert(
        "IOutputStream".to_owned(),
        Interface {
            c_vtbl: "winasync_Windows_Storage_Streams_IOutputStreamVtbl".to_owned(),
            winrt: "winrt::Windows::Storage::Streams::IOutputStream".to_owned(),
            winrt_guard: "WINRT_Windows_Storage_Streams_H",
            slots: vec![
                slot("WriteAsync", &["void*", "void*", "void**"]),
                slot("FlushAsync", &["void*", "void**"]),
            ],
        },
    );
    m
}

/// Every `I*_Vtbl` struct and every `Interface for <Class>` impl, at any
/// module depth.
fn collect_bindings(
    items: &[syn::Item],
    interfaces: &mut BTreeMap<String, Interface>,
    default_interface: &mut BTreeMap<String, String>,
) {
    for item in items {
        match item {
            syn::Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    collect_bindings(items, interfaces, default_interface);
                }
            }
            syn::Item::Struct(s) => {
                let name = s.ident.to_string();
                let Some(iface) = name.strip_suffix("_Vtbl") else { continue };
                let mut slots = Vec::new();
                for field in &s.fields {
                    let fname = field.ident.as_ref().expect("named vtable field").to_string();
                    if fname == "base__" {
                        let base = quote_type(&field.ty);
                        assert!(
                            base.ends_with("IInspectable_Vtbl"),
                            "{name}: base is `{base}`, only IInspectable-based interfaces are expected"
                        );
                        continue;
                    }
                    let syn::Type::BareFn(f) = &field.ty else {
                        panic!("{name}.{fname}: not a function pointer")
                    };
                    let params = f.inputs.iter().map(|arg| c_type(&arg.ty, &format!("{name}.{fname}"))).collect();
                    let ret = match &f.output {
                        syn::ReturnType::Type(_, t) => quote_type(t),
                        syn::ReturnType::Default => "()".to_owned(),
                    };
                    assert!(ret.ends_with("HRESULT"), "{name}.{fname}: returns `{ret}`, not HRESULT");
                    slots.push(Slot { name: fname, params });
                }
                interfaces.insert(
                    iface.to_owned(),
                    Interface {
                        c_vtbl: format!("{PREFIX}{iface}Vtbl"),
                        winrt: format!("winrt::{}::{iface}", NAMESPACE.replace('.', "::")),
                        winrt_guard: "WINRT_Pbk20191_WinAsync_Comio_H",
                        slots,
                    },
                );
            }
            syn::Item::Impl(i) => {
                let Some((_, trait_path, _)) = &i.trait_ else { continue };
                if trait_path.segments.last().map(|s| s.ident.to_string()).as_deref() != Some("Interface") {
                    continue;
                }
                let syn::Type::Path(self_ty) = &*i.self_ty else { continue };
                let class = self_ty.path.segments.last().unwrap().ident.to_string();
                for item in &i.items {
                    let syn::ImplItem::Type(t) = item else { continue };
                    if t.ident != "Vtable" {
                        continue;
                    }
                    // `<IFoo as windows_core::Interface>::Vtable`
                    let syn::Type::Path(p) = &t.ty else { continue };
                    let Some(qself) = &p.qself else { continue };
                    let syn::Type::Path(inner) = &*qself.ty else { continue };
                    let iface = inner.path.segments.last().unwrap().ident.to_string();
                    default_interface.insert(class.clone(), iface);
                }
            }
            _ => {}
        }
    }
}

/// The `#[implement(..., Export = <PREFIX>..., ...)]` lists under `dir`.
fn collect_classes(dir: &Path, root: &Path, default_interface: &BTreeMap<String, String>, out: &mut Vec<Class>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_classes(&path, root, default_interface, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("bindings.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let file = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let source = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for item in &file.items {
            let syn::Item::Struct(s) = item else { continue };
            for attr in &s.attrs {
                if attr.path().segments.last().unwrap().ident != "implement" {
                    continue;
                }
                let args = attr
                    .parse_args_with(syn::punctuated::Punctuated::<Entry, syn::Token![,]>::parse_terminated)
                    .unwrap_or_else(|e| panic!("{}: #[implement] on {}: {e}", path.display(), s.ident));
                let mut prefix = None;
                let mut listed = Vec::new();
                for arg in &args {
                    match arg {
                        Entry::Option { key, value } => {
                            if key == "Export" {
                                prefix = Some(quote::ToTokens::to_token_stream(value).to_string());
                            }
                        }
                        Entry::Interface(syn::Type::Path(p)) => {
                            listed.push(p.path.segments.last().unwrap().ident.to_string());
                        }
                        Entry::Interface(other) => panic!(
                            "{}: #[implement] on {}: unexpected entry `{}`",
                            path.display(),
                            s.ident,
                            quote_type(other)
                        ),
                    }
                }
                let Some(prefix) = prefix else { continue };
                if !prefix.starts_with(PREFIX) {
                    // A test class, or another component's.
                    continue;
                }
                let class = s.ident.to_string();
                let getters = listed
                    .iter()
                    .map(|entry| {
                        if *entry == class {
                            let interface = default_interface
                                .get(&class)
                                .unwrap_or_else(|| panic!("{class}: no `Interface for {class}` impl in bindings.rs"));
                            Getter {
                                symbol: format!("{prefix}_vtable"),
                                interface: interface.clone(),
                            }
                        } else {
                            Getter {
                                symbol: format!("{prefix}_{entry}_vtable"),
                                interface: entry.clone(),
                            }
                        }
                    })
                    .collect();
                out.push(Class {
                    name: class,
                    source: source.clone(),
                    getters,
                });
            }
        }
    }
}

/// One entry of an `#[implement(...)]` list: an interface type (possibly
/// generic, which is why these are not expressions) or `Key = value`.
enum Entry {
    Interface(syn::Type),
    Option { key: String, value: syn::Expr },
}

impl syn::parse::Parse for Entry {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.peek(syn::Ident) && input.peek2(syn::Token![=]) {
            let key: syn::Ident = input.parse()?;
            let _: syn::Token![=] = input.parse()?;
            let value: syn::Expr = input.parse()?;
            Ok(Entry::Option {
                key: key.to_string(),
                value,
            })
        } else {
            Ok(Entry::Interface(input.parse()?))
        }
    }
}

fn quote_type(t: &syn::Type) -> String {
    quote::ToTokens::to_token_stream(t).to_string().replace(' ', "")
}

/// A bindgen ABI type as C.
fn c_type(t: &syn::Type, at: &str) -> String {
    match t {
        syn::Type::Ptr(p) => {
            let inner = c_type(&p.elem, at);
            if p.mutability.is_some() {
                format!("{inner}*")
            } else {
                format!("const {inner}*")
            }
        }
        syn::Type::Path(p) => {
            let last = p.path.segments.last().unwrap().ident.to_string();
            match last.as_str() {
                "c_void" => "void",
                "u8" => "UINT8",
                "u16" => "UINT16",
                "u32" => "UINT32",
                "u64" => "UINT64",
                "i8" => "INT8",
                "i16" => "INT16",
                "i32" => "INT32",
                "i64" => "INT64",
                "f32" => "float",
                "f64" => "double",
                "usize" => "UINT_PTR",
                "isize" => "INT_PTR",
                "bool" => "bool",
                "HRESULT" => "HRESULT",
                "GUID" => "GUID",
                "HSTRING" => "HSTRING",
                // By value: one INT64, declared in the header's prelude.
                "TimeSpan" => "winasync_Windows_Foundation_TimeSpan",
                "DateTime" => "winasync_Windows_Foundation_DateTime",
                other => panic!("{at}: no C spelling for `{other}` -- add it to winmdgen's header generator"),
            }
            .to_owned()
        }
        other => panic!("{at}: unexpected type `{}`", quote_type(other)),
    }
}

fn render(classes: &[Class], used: &BTreeMap<&str, &Interface>) -> String {
    let mut s = String::new();
    s.push_str(
        "// winasync's WinRT component as a static library: the declarations.
//
// Generated by `cargo run -p winmdgen` from the vtable layouts in
// src/comio/bindings.rs and the `Export =` lists in src/comio; regenerate
// rather than edit.
//
// Link `winasync_static.lib` (cargo build -p winasync-static --release) plus
// the libraries `cargo rustc -p winasync-static -- --print native-static-libs`
// names, and runtimeobject.lib for HSTRING. Nothing here is a reserved name:
// no Dll*, no Ro*. See static/src/lib.rs.
//
// Each getter returns the class's vtable for one interface -- the same table
// its objects carry. A method called through it takes the object's interface
// pointer for that interface, exactly as the vtable call would; the C++
// wrappers below take that pointer as `void* self`, and, when the C++/WinRT
// projection is included first, the projected interface itself. Under full
// LTO (clang -flto with rustc -Clto=fat -Clinker-plugin-lto, caller target
// features covering rustc's x86-64 baseline) the call inlines.
#pragma once
#include <windows.h>
#include <hstring.h>
#ifndef __cplusplus
#include <stdbool.h>
#endif

#ifdef __cplusplus
extern \"C\" {
#endif

// The activation factory for a runtime class name, as DllGetActivationFactory
// would hand it out: IActivationFactory, refcount 1, in `factory`.
HRESULT __stdcall winasync_get_activation_factory(HSTRING name, void** factory);

// The class object for a CLSID, as DllGetClassObject would hand it out.
HRESULT __stdcall winasync_get_class_object(const GUID* clsid, const GUID* iid, void** object);

// Every WinRT vtable begins with IInspectable's six slots.
typedef struct winasync_IInspectableVtbl {
    HRESULT (__stdcall *QueryInterface)(void* self, const GUID* iid, void** object);
    ULONG (__stdcall *AddRef)(void* self);
    ULONG (__stdcall *Release)(void* self);
    HRESULT (__stdcall *GetIids)(void* self, ULONG* count, GUID** iids);
    HRESULT (__stdcall *GetRuntimeClassName)(void* self, HSTRING* name);
    HRESULT (__stdcall *GetTrustLevel)(void* self, INT32* level);
} winasync_IInspectableVtbl;

// Windows.Foundation.TimeSpan and DateTime, passed by value: 100 ns units.
typedef struct winasync_Windows_Foundation_TimeSpan { INT64 Duration; } winasync_Windows_Foundation_TimeSpan;
typedef struct winasync_Windows_Foundation_DateTime { INT64 UniversalTime; } winasync_Windows_Foundation_DateTime;
",
    );

    for (name, i) in used {
        let _ = writeln!(s, "\n// {name}");
        let _ = writeln!(s, "typedef struct {} {{", i.c_vtbl);
        s.push_str("    winasync_IInspectableVtbl base;\n");
        for slot in &i.slots {
            let _ = writeln!(s, "    HRESULT (__stdcall *{})({});", slot.name, c_params(slot));
        }
        let _ = writeln!(s, "}} {};", i.c_vtbl);
    }

    for c in classes {
        let _ = writeln!(s, "\n// {NAMESPACE}.{} ({})", c.name, c.source);
        for g in &c.getters {
            let i = used[g.interface.as_str()];
            let _ = writeln!(s, "const {}* __stdcall {}(void);", i.c_vtbl, g.symbol);
        }
    }

    s.push_str(
        "
#ifdef __cplusplus
}  // extern \"C\"

// C++: each class's methods as functions that call through its table.
// `self` is the object's pointer for that interface -- from C++/WinRT,
// `winrt::get_abi(object.as<I>())` -- and the overloads taking the projected
// interface exist when its C++/WinRT header was included before this one.
namespace winasync {
",
    );
    for c in classes {
        let _ = writeln!(s, "namespace {} {{", c.name);
        for g in &c.getters {
            let i = used[g.interface.as_str()];
            let _ = writeln!(s, "    // {}", g.interface);
            for slot in &i.slots {
                let (decl, call) = cpp_params(slot);
                let _ = writeln!(
                    s,
                    "    inline HRESULT {}({}) {{ return ::{}()->{}({}); }}",
                    slot.name, decl, g.symbol, slot.name, call
                );
            }
            let _ = writeln!(s, "#ifdef {}", i.winrt_guard);
            for slot in &i.slots {
                let (decl, call) = cpp_params_winrt(slot, &i.winrt);
                let _ = writeln!(
                    s,
                    "    inline HRESULT {}({}) {{ return {}({}); }}",
                    slot.name, decl, slot.name, call
                );
            }
            s.push_str("#endif\n");
        }
        s.push_str("}\n");
    }
    s.push_str("}  // namespace winasync\n#endif\n");
    s
}

/// `void* self, void* a1, UINT32 a2`.
fn c_params(slot: &Slot) -> String {
    slot.params
        .iter()
        .enumerate()
        .map(|(n, t)| if n == 0 { format!("{t} self") } else { format!("{t} a{n}") })
        .collect::<Vec<_>>()
        .join(", ")
}

fn cpp_params(slot: &Slot) -> (String, String) {
    let decl = c_params(slot);
    let call = (0..slot.params.len())
        .map(|n| if n == 0 { "self".to_owned() } else { format!("a{n}") })
        .collect::<Vec<_>>()
        .join(", ");
    (decl, call)
}

fn cpp_params_winrt(slot: &Slot, winrt: &str) -> (String, String) {
    let mut decl = vec![format!("{winrt} const& self")];
    let mut call = vec!["winrt::get_abi(self)".to_owned()];
    for (n, t) in slot.params.iter().enumerate().skip(1) {
        decl.push(format!("{t} a{n}"));
        call.push(format!("a{n}"));
    }
    (decl.join(", "), call.join(", "))
}
