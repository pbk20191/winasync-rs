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
pub fn buffer_ptr(buf: &IBuffer) -> windows_core::Result<*mut u8> {
    let access: IBufferByteAccess = buf.cast()?;
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
