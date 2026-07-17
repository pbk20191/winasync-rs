use windows::Storage::Streams::{Buffer, IBuffer};
use windows::Win32::System::WinRT::IBufferByteAccess;
use windows_core::Interface;

pub(crate) fn new_buffer(capacity: u32) -> windows_core::Result<Buffer> {
    Buffer::Create(capacity)
}

pub(crate) fn buffer_ptr(buf: &IBuffer) -> windows_core::Result<*mut u8> {
    let access: IBufferByteAccess = buf.cast()?;
    unsafe { access.Buffer() }
}

pub(crate) fn buffer_capacity(buf: &IBuffer) -> windows_core::Result<u32> {
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
