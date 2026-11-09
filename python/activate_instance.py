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

_ActivateInstance = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)
)

def activate(cls):
    factory_capsule = get_activation_factory(cls, _IACTIVATION_FACTORY)
    factory = _py.PyCapsule_GetPointer(factory_capsule, _CAPSULE_NAME)
    vtable = ctypes.cast(factory, ctypes.POINTER(ctypes.c_void_p))[0]
    slot = ctypes.cast(vtable, ctypes.POINTER(ctypes.c_void_p))[6]
    instance = ctypes.c_void_p()
    hr = _ActivateInstance(slot)(factory, ctypes.byref(instance))
    if hr < 0:
        raise OSError(None, "ActivateInstance failed", None, hr & 0xFFFFFFFF)
    capsule = _py.PyCapsule_New(instance, _CAPSULE_NAME, None)
    return wrap_interface(capsule, cls)
