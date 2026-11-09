use windows::Storage::Streams::{Buffer, IBuffer};
use windows::Win32::System::WinRT::IBufferByteAccess;
use windows_core::Interface;

/// Allocates a WinRT `IBuffer` of `capacity` bytes.
///
/// Public because the streams in this module speak `IBuffer`: a caller of
/// `ReadAsync`/`WriteAsync` has to be able to make one.
pub fn new_buffer(capacity: u32) -> windows_core::Result<Buffer> {
    Buffer::Create(capacity)
}

/// The raw bytes behind an `IBuffer`, via `IBufferByteAccess`.
///
/// # Safety
/// Safe to call, but the returned pointer is only valid while `buf` lives,
/// and only `Capacity()` bytes are addressable — `Length()` says how many
/// currently hold data.
///
/// # A buffer without byte access
///
/// `E_INVALIDARG`, with a message, rather than the bare `E_NOINTERFACE` the
/// cast fails with -- because the cause is almost never the buffer's
/// implementation and almost always where it came from.
///
/// `IBufferByteAccess` is in-process only: it has no proxy, by design, since
/// a raw pointer means nothing in another address space. A buffer reaches the
/// out-of-process server (`comio::server`) with byte access only if it
/// *marshals by value*, which the system `Buffer` does through the buffer
/// marshaler (`RoGetBufferMarshaler`) and which is the Windows Runtime's
/// contract for any `IBuffer` meant to cross processes. A buffer that does
/// not arrives as a proxy, and there is no way to reach its bytes through the
/// `IBuffer` members alone -- so this is an error to report, not a case to
/// work around.
pub fn buffer_ptr(buf: &IBuffer) -> windows_core::Result<*mut u8> {
    let access: IBufferByteAccess = buf.cast().map_err(|_| {
        windows_core::Error::new(
            windows::Win32::Foundation::E_INVALIDARG,
            "the IBuffer has no IBufferByteAccess; across a process boundary \
             only a buffer that marshals by value (such as \
             Windows.Storage.Streams.Buffer) carries its bytes",
        )
    })?;
    unsafe { access.Buffer() }
}

/// How many bytes the buffer can hold (not how many it currently holds).
pub fn buffer_capacity(buf: &IBuffer) -> windows_core::Result<u32> {
    buf.Capacity()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buffer_ptr_writable() {
        let buf = new_buffer(8).unwrap();
        let ib: windows::Storage::Streams::IBuffer = buf.clone().into();
        let p = buffer_ptr(&ib).unwrap();
        unsafe { *p = 0xAB; }
        assert_eq!(buffer_capacity(&ib).unwrap(), 8);
        assert!(!p.is_null());
    }
}
