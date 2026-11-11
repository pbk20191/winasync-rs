use super::*;

/// Activation end to end inside one process, minus the loader: ask for
/// each class object the way COM does, make an instance, and check it is
/// the class that was asked for.
///
/// This is the part of the path that has no registry in it. The loader's
/// half — registry lookup, `LoadLibrary`, `GetProcAddress` — is covered
/// by `tests/activation.rs`, which can only run against the real DLL.
#[test]
fn every_class_activates_through_its_class_object() {
    // Each class is identified by an interface only it implements, so a
    // factory wired to the wrong constructor cannot pass.
    type Identified = (GUID, fn(&IUnknown) -> bool);
    let expected: &[Identified] = &[
        (CLSID_NAMED_PIPE_CONNECTION_LISTENER, |u| {
            u.cast::<super::super::INamedPipeBind>().is_ok()
        }),
        (CLSID_UNIX_CONNECTION_LISTENER, |u| {
            u.cast::<super::super::IUnixBind>().is_ok()
        }),
        (CLSID_HYPER_V_CONNECTION_LISTENER, |u| {
            u.cast::<super::super::IHyperVBind>().is_ok()
        }),
        (CLSID_NAMED_PIPE_CONNECTOR, |u| {
            u.cast::<super::super::INamedPipeConnector>().is_ok()
        }),
        (CLSID_UNIX_CONNECTOR, |u| {
            u.cast::<super::super::IUnixConnector>().is_ok()
        }),
        (CLSID_HYPER_V_CONNECTOR, |u| {
            u.cast::<super::super::IHyperVConnector>().is_ok()
        }),
        #[cfg(feature = "http")]
        (CLSID_HTTP_SERVER, |u| u.cast::<rc::IHttpServer>().is_ok()),
    ];
    assert_eq!(expected.len(), CLASSES.len(), "a class has no coverage here");

    for (clsid, is_right_class) in expected {
        let factory: IClassFactory = unsafe {
            let mut raw = core::ptr::null_mut();
            dll_get_class_object(clsid, &IClassFactory::IID, &mut raw)
                .ok()
                .expect("DllGetClassObject");
            IClassFactory::from_raw(raw)
        };

        let instance: IUnknown = unsafe { factory.CreateInstance(None) }.expect("CreateInstance");
        assert!(
            is_right_class(&instance),
            "{} activated the wrong class",
            clsid_key(clsid)
        );

        // A second instance is a second object, not the same one again.
        let other: IUnknown = unsafe { factory.CreateInstance(None) }.expect("CreateInstance");
        assert_ne!(instance.as_raw(), other.as_raw());
    }
}

/// The refusals a class factory owes its caller.
#[test]
fn activation_refuses_what_it_cannot_do() {
    let mut raw = core::ptr::null_mut();

    // A CLSID this module does not serve. COM relies on this exact code
    // to go on looking elsewhere rather than failing the activation.
    let stranger = GUID::from_u128(0x00000000_0000_0000_0000_0000000000ff);
    assert_eq!(
        unsafe { dll_get_class_object(&stranger, &IClassFactory::IID, &mut raw) },
        CLASS_E_CLASSNOTAVAILABLE
    );
    assert!(raw.is_null());

    // An interface the class object does not have.
    assert!(
        unsafe {
            dll_get_class_object(
                &CLSID_NAMED_PIPE_CONNECTOR,
                &super::super::IUnixConnector::IID,
                &mut raw,
            )
        }
        .is_err()
    );
    assert!(raw.is_null());

    // Aggregation, which these objects cannot support.
    let factory: IClassFactory = unsafe {
        let mut f = core::ptr::null_mut();
        dll_get_class_object(&CLSID_UNIX_CONNECTOR, &IClassFactory::IID, &mut f)
            .ok()
            .unwrap();
        IClassFactory::from_raw(f)
    };
    let outer: IUnknown = super::super::NamedPipeConnector::new().into_interface();
    let err = unsafe { factory.CreateInstance::<_, IUnknown>(&outer) }
        .expect_err("aggregation must be refused");
    assert_eq!(err.code(), CLASS_E_NOAGGREGATION);

    // Locking is accepted even though it does nothing.
    unsafe { factory.LockServer(true) }.expect("LockServer");
    unsafe { factory.LockServer(false) }.expect("LockServer");
}
