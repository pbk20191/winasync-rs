//! Async `DeviceIoControl` over TP_IO — the escape hatch for everything the
//! typed transports in this module do not cover.
//!
//! A control code is how Windows exposes the rest of the kernel's IO surface:
//! `FSCTL_*` on files and volumes (sparse ranges, reparse points, compression,
//! USN journal), `IOCTL_DISK_*` on drives, `IOCTL_AFD_*` under the socket
//! layer, and whatever a third-party driver defines. Every one of them is just
//! `(code, input buffer, output buffer)`, and every one can be issued
//! overlapped — so a single generic op covers the lot.
//!
//! [`Device`] owns the handle and the registration; the op is identical to a
//! file read: issue, hand the buffers to the kernel, resolve with the number of
//! bytes the driver wrote into the output buffer.

use super::{submit, IoRes, Issue, OpFuture};
use crate::tp::Runtime;
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

/// Input and output buffers of an in-flight ioctl. Both live in the op block
/// for its whole duration — the driver may write into `output` at any point
/// before completion.
struct IoctlPayload {
    input: Vec<u8>,
    output: Vec<u8>,
}

/// A handle opened for overlapped `DeviceIoControl`.
///
/// Open a device by its namespace path (`\\.\PhysicalDrive0`, `\\.\C:`,
/// `\\.\PipeName`) or any file or directory whose driver answers the control
/// codes you want to send.
pub struct Device {
    res: Arc<IoRes>,
}

impl Device {
    /// Opens `path` with `FILE_FLAG_OVERLAPPED` and read/write access.
    ///
    /// Volume and physical-drive paths generally need an elevated process;
    /// plain files and directories do not.
    pub fn open(rt: &Runtime, path: &str) -> windows_core::Result<Device> {
        Self::open_with(rt, path, GENERIC_READ.0 | GENERIC_WRITE.0)
    }

    /// Opens `path` for metadata-only control codes — access `0`, which is what
    /// `FSCTL_*` codes carrying `FILE_ANY_ACCESS` need and is granted where a
    /// read/write open would be denied.
    pub fn open_query(rt: &Runtime, path: &str) -> windows_core::Result<Device> {
        Self::open_with(rt, path, 0)
    }

    fn open_with(rt: &Runtime, path: &str, access: u32) -> windows_core::Result<Device> {
        let handle = unsafe {
            CreateFileW(
                &windows_core::HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )?
        };
        match IoRes::register(&rt.inner, handle, false) {
            Ok(res) => Ok(Device { res }),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    /// Issues control code `code` with `input`, filling `output`.
    ///
    /// Resolves to the number of bytes the driver wrote into `output`, plus
    /// both buffers handed back (the input one too — a control code may be
    /// re-issued with the same request, and the allocation is worth keeping).
    /// Pass an empty `Vec` for a direction the code does not use.
    ///
    /// A driver that needs a bigger output buffer reports
    /// `ERROR_MORE_DATA` / `ERROR_INSUFFICIENT_BUFFER`; retry with a larger
    /// one.
    pub fn ioctl(
        &self,
        code: u32,
        input: Vec<u8>,
        output: Vec<u8>,
    ) -> impl Future<Output = (windows_core::Result<usize>, Vec<u8>, Vec<u8>)> + Send + use<> {
        let handle = self.res.handle;
        let fut: OpFuture<IoctlPayload> = unsafe {
            submit(&self.res, IoctlPayload { input, output }, move |ovl, p| {
                // Empty buffers must be passed as NULL, not as a dangling
                // one-past-the-end pointer: some drivers validate the pointer
                // even for a zero length.
                let in_ptr = if p.input.is_empty() {
                    None
                } else {
                    Some(p.input.as_ptr() as *const core::ffi::c_void)
                };
                let out_ptr = if p.output.is_empty() {
                    None
                } else {
                    Some(p.output.as_mut_ptr() as *mut core::ffi::c_void)
                };
                match DeviceIoControl(
                    handle,
                    code,
                    in_ptr,
                    p.input.len() as u32,
                    out_ptr,
                    p.output.len() as u32,
                    None,
                    Some(ovl),
                ) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        async move {
            let (result, payload) = fut.await;
            (result, payload.input, payload.output)
        }
    }
}

/// `CTL_CODE` from winioctl.h — assembles a control code from its parts, so a
/// driver's codes can be written out as the header spells them.
pub const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

/// `FILE_DEVICE_FILE_SYSTEM` — the device type of every `FSCTL_*` code.
pub const FILE_DEVICE_FILE_SYSTEM: u32 = 0x0000_0009;
/// `METHOD_BUFFERED` — the transfer method of most control codes.
pub const METHOD_BUFFERED: u32 = 0;
/// `FILE_ANY_ACCESS` — issuable on a handle opened with no access rights.
pub const FILE_ANY_ACCESS: u32 = 0;

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, timeout, Runtime};
    use super::*;
    use std::time::Duration;

    /// `FSCTL_GET_COMPRESSION` — reads back a USHORT compression format, needs
    /// no access rights, and works on any file on any volume.
    const FSCTL_GET_COMPRESSION: u32 =
        ctl_code(FILE_DEVICE_FILE_SYSTEM, 15, METHOD_BUFFERED, FILE_ANY_ACCESS);

    #[test]
    fn ioctl_reads_compression_state() {
        let rt = Runtime::new();
        let path = std::env::temp_dir().join(format!("tp_ioctl_{}.bin", std::process::id()));
        std::fs::write(&path, b"ioctl").unwrap();
        let path_str = path.to_string_lossy().into_owned();

        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let dev = Device::open_query(&rt, &path_str).unwrap();
            let (n, _in_buf, out) = dev
                .ioctl(FSCTL_GET_COMPRESSION, Vec::new(), vec![0u8; 2])
                .await;
            let n = n.unwrap();
            assert_eq!(n, 2, "FSCTL_GET_COMPRESSION returns a USHORT");
            // COMPRESSION_FORMAT_NONE on a freshly written temp file.
            assert_eq!(u16::from_ne_bytes([out[0], out[1]]), 0);
        }));
        let _ = std::fs::remove_file(&path);
        result.expect("ioctl timed out");
    }

    #[test]
    fn ctl_code_matches_winioctl() {
        // FSCTL_GET_COMPRESSION is 0x9003C in winioctl.h.
        assert_eq!(FSCTL_GET_COMPRESSION, 0x9003C);
        // FSCTL_SET_SPARSE = CTL_CODE(9, 49, METHOD_BUFFERED, FILE_SPECIAL_ACCESS)
        assert_eq!(ctl_code(FILE_DEVICE_FILE_SYSTEM, 49, 0, 0), 0x900C4);
    }

    #[test]
    fn ioctl_bad_code_fails_cleanly() {
        let rt = Runtime::new();
        let path = std::env::temp_dir().join(format!("tp_ioctl_bad_{}.bin", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        let path_str = path.to_string_lossy().into_owned();
        let result = block_on(timeout(&rt, Duration::from_secs(10), async {
            let dev = Device::open_query(&rt, &path_str).unwrap();
            // Function 4095 of the file-system device is not implemented.
            let code = ctl_code(FILE_DEVICE_FILE_SYSTEM, 4095, METHOD_BUFFERED, FILE_ANY_ACCESS);
            let (r, _, _) = dev.ioctl(code, Vec::new(), vec![0u8; 8]).await;
            assert!(r.is_err(), "an unimplemented control code must fail");
        }));
        let _ = std::fs::remove_file(&path);
        result.expect("ioctl error path timed out");
    }
}
