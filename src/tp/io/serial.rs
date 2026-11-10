//! Serial ports (`COM1`, `\\.\COM10`) over TP_IO: overlapped reads and writes,
//! line settings, and `WaitCommEvent` as a future.
//!
//! `tp` only. WinRT already has `Windows.Devices.SerialCommunication.
//! SerialDevice`, so `comio` -- which exists for what WinRT lacks -- has no
//! serial port of its own.
//!
//! Verified only as far as the host allows: CI runners have no serial ports,
//! so the tests that need one run when `WINASYNC_TEST_COM` names a port and
//! are skipped otherwise. See `tests` below.

use super::{submit, IoRes, Issue};
use crate::common::buffer::{IoBuf, IoBufMut};
use crate::tp::Runtime;
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows_core::HSTRING;

pub use windows::Win32::Devices::Communication::{COMMTIMEOUTS, COMM_EVENT_MASK, DCB};
use windows::Win32::Devices::Communication::{
    GetCommState, SetCommMask, SetCommState, SetCommTimeouts, WaitCommEvent,
};

/// An open serial port.
pub struct SerialPort {
    res: Arc<IoRes>,
}

impl SerialPort {
    /// Opens `name` -- `COM3`, or a full device path such as `\\.\COM10`
    /// (ports above 9 need the prefix, which this adds when it is missing).
    ///
    /// Opened exclusively, as Windows requires of a comm device.
    pub fn open(rt: &Runtime, name: &str) -> windows_core::Result<SerialPort> {
        let path = if name.starts_with(r"\\") {
            name.to_owned()
        } else {
            format!(r"\\.\{name}")
        };
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )?
        };
        match IoRes::register(&rt.inner, handle, false) {
            Ok(res) => Ok(SerialPort { res }),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    /// The line settings: baud rate, byte size, parity, stop bits, flow
    /// control (`GetCommState`).
    pub fn state(&self) -> windows_core::Result<DCB> {
        let mut dcb = DCB {
            DCBlength: size_of::<DCB>() as u32,
            ..Default::default()
        };
        unsafe { GetCommState(self.res.handle, &mut dcb)? };
        Ok(dcb)
    }

    /// Changes the line settings (`SetCommState`). Start from
    /// [`state`](Self::state) and change what you need.
    pub fn set_state(&self, dcb: &DCB) -> windows_core::Result<()> {
        unsafe { SetCommState(self.res.handle, dcb) }
    }

    /// When a read or write completes (`SetCommTimeouts`).
    ///
    /// These decide what a read *means*: with every field zero a read waits
    /// until its buffer is full. For "return what has arrived, waiting up to
    /// `n` ms for the first byte", set `ReadIntervalTimeout` and
    /// `ReadTotalTimeoutMultiplier` to `u32::MAX` and
    /// `ReadTotalTimeoutConstant` to `n`.
    pub fn set_timeouts(&self, timeouts: &COMMTIMEOUTS) -> windows_core::Result<()> {
        unsafe { SetCommTimeouts(self.res.handle, timeouts) }
    }

    /// Reads into `buf`, as the timeouts decide; resolves to the byte count
    /// and hands the buffer back.
    pub fn read<B: IoBufMut + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle;
        let fut = unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let slice =
                    core::slice::from_raw_parts_mut(buf.stable_mut_ptr(), IoBuf::bytes_total(buf));
                classify(ReadFile(handle, Some(slice), None, Some(ovl)))
            })
        };
        async move {
            let (result, buf) = fut.await;
            let n = result.as_ref().ok().copied();
            (result, crate::common::buffer::mark_read(n, buf))
        }
    }

    /// Writes `buf`; resolves to the byte count and hands the buffer back.
    pub fn write<B: IoBuf + 'static>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle;
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let slice = core::slice::from_raw_parts(buf.stable_ptr(), IoBuf::len(buf));
                classify(WriteFile(handle, Some(slice), None, Some(ovl)))
            })
        }
    }

    /// Waits for one of the events in `mask` (`EV_RXCHAR`, `EV_CTS`,
    /// `EV_TXEMPTY`, ...) and resolves to the ones that occurred.
    ///
    /// Sets the port's event mask, then `WaitCommEvent`. One wait at a time:
    /// the mask belongs to the port, and setting it while a wait is pending
    /// ends that wait with an empty set -- so a second `wait_event` cuts the
    /// first one short. Dropping the future cancels the wait.
    pub fn wait_event(
        &self,
        mask: COMM_EVENT_MASK,
    ) -> impl Future<Output = windows_core::Result<COMM_EVENT_MASK>> + Send + use<> {
        let handle = self.res.handle;
        let set = unsafe { SetCommMask(handle, mask) };
        // The kernel writes the events into the op block's payload, which
        // stays put until the wait completes.
        let fut = set.is_ok().then(|| unsafe {
            submit(&self.res, COMM_EVENT_MASK(0), move |ovl, events| {
                classify(WaitCommEvent(handle, events, Some(ovl)))
            })
        });
        async move {
            set?;
            let (result, events) = fut.expect("the mask was set").await;
            result?;
            Ok(events)
        }
    }
}

/// Every overlapped call here has the same three outcomes.
fn classify(r: windows_core::Result<()>) -> Issue {
    match r {
        Ok(()) => Issue::SyncSuccess,
        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
        Err(e) => Issue::Failed(e.code()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, timeout};
    use super::*;
    use std::time::Duration;
    use windows::Win32::Devices::Communication::{EV_RXCHAR, EV_TXEMPTY};

    /// The port the host offers for testing, if any. Never on CI: the
    /// runners have no serial hardware.
    fn test_port() -> Option<String> {
        if std::env::var_os("GITHUB_ACTIONS").is_some() {
            return None;
        }
        let port = std::env::var("WINASYNC_TEST_COM").ok();
        if port.is_none() {
            eprintln!("skipped: set WINASYNC_TEST_COM to a serial port (e.g. COM3) to run this");
        }
        port
    }

    #[test]
    fn a_missing_port_is_not_found() {
        let rt = Runtime::new();
        let err = SerialPort::open(&rt, "COM250").err().unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult());
    }

    /// Settings round-trip, and a write is followed by `EV_TXEMPTY` once the
    /// port has sent it -- which needs no peer on the other end.
    #[test]
    fn host_port_settings_and_tx_empty() {
        let Some(name) = test_port() else { return };
        let rt = Runtime::new();
        let port = SerialPort::open(&rt, &name).unwrap();
        let mut dcb = port.state().unwrap();
        dcb.BaudRate = 9600;
        dcb.ByteSize = 8;
        port.set_state(&dcb).unwrap();
        assert_eq!(port.state().unwrap().BaudRate, 9600);

        let wait = port.wait_event(EV_TXEMPTY);
        block_on(port.write(b"U".to_vec())).0.unwrap();
        let events = block_on(timeout(&rt, Duration::from_secs(5), wait))
            .expect("no EV_TXEMPTY within 5 s")
            .unwrap();
        assert!(events.0 & EV_TXEMPTY.0 != 0, "events {:#x}", events.0);
    }

    /// A wait for input that never comes is cancelled by dropping it.
    #[test]
    fn host_port_wait_is_cancelled_by_drop() {
        let Some(name) = test_port() else { return };
        let rt = Runtime::new();
        let port = SerialPort::open(&rt, &name).unwrap();
        let waited = block_on(timeout(&rt, Duration::from_millis(200), port.wait_event(EV_RXCHAR)));
        // Something may really arrive on a live line; only a hang is wrong.
        eprintln!("EV_RXCHAR wait -> {:?}", waited.as_ref().map(|r| r.as_ref().map(|e| e.0)));
    }
}
