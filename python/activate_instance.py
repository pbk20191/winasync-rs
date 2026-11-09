"""Create an instance of a projected class that has no Python constructor.

`windows-rdl` cannot emit a `.ctor` MethodDef on a runtime class, so PyWinRT
projects our classes without `__new__`. Everything else about them projects
correctly, so this reaches the activation factory the projection already
knows how to find and calls `IActivationFactory::ActivateInstance` (vtable
slot 6) on it, then hands the resulting pointer back to the runtime.
"""

import ctypes
import uuid

from winrt.runtime.interop import get_activation_factory, wrap_interface

_IACTIVATION_FACTORY = uuid.UUID("00000035-0000-0000-C000-000000000046")
_CAPSULE_NAME = b"winrt.interface"

_py = ctypes.pythonapi
_py.PyCapsule_GetPointer.restype = ctypes.c_void_p
_py.PyCapsule_GetPointer.argtypes = [ctypes.py_object, ctypes.c_char_p]
_py.PyCapsule_New.restype = ctypes.py_object
_py.PyCapsule_New.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_void_p]

# `IUnknown`: QueryInterface, AddRef, Release, then `IInspectable`'s three,
# so a runtime class's own members start at slot 6 — which is where
# `IActivationFactory::ActivateInstance` sits too.
_ADD_REF_SLOT = 1
_RELEASE_SLOT = 2
_ACTIVATE_INSTANCE_SLOT = 6

_ActivateInstanceFn = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)
)
_ReleaseFn = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)


def _vtable_slot(interface, index):
    vtable = ctypes.cast(interface, ctypes.POINTER(ctypes.c_void_p))[0]
    return ctypes.cast(vtable, ctypes.POINTER(ctypes.c_void_p))[index]


def _release(interface):
    _ReleaseFn(_vtable_slot(interface, _RELEASE_SLOT))(interface)


def activate(cls):
    """A new instance of `cls`, as `cls()` would give if it had a constructor."""
    factory_capsule = get_activation_factory(cls, _IACTIVATION_FACTORY)
    factory = _py.PyCapsule_GetPointer(factory_capsule, _CAPSULE_NAME)

    instance = ctypes.c_void_p()
    activate_instance = _ActivateInstanceFn(
        _vtable_slot(factory, _ACTIVATE_INSTANCE_SLOT)
    )
    hr = activate_instance(factory, ctypes.byref(instance))
    if hr < 0:
        raise OSError(None, "ActivateInstance failed", None, hr & 0xFFFFFFFF)

    # `wrap_interface` takes a reference of its own rather than consuming the
    # capsule's -- which is what a `winrt.interface` capsule promises, since
    # it releases what it holds when collected -- so the one
    # `ActivateInstance` handed us is ours to drop. Skipping this leaks
    # exactly one reference per activation; measured, not assumed, by reading
    # the count through AddRef/Release before and after.
    #
    # Done here rather than from a capsule destructor because a ctypes
    # callback taking `py_object` mishandles the borrowed reference it is
    # given, and releases the wrong pointer.
    wrapped = wrap_interface(
        _py.PyCapsule_New(instance, _CAPSULE_NAME, None), cls
    )
    _release(instance)
    return wrapped
