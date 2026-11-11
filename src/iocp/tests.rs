use super::*;
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent};

struct Event(OwnedHandle);
impl Event {
    fn new() -> Self {
        let h = unsafe { CreateEventW(None, true, false, None).unwrap() };
        Event(unsafe { OwnedHandle::from_raw_handle(h.0) })
    }
    fn raw(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
    fn set(&self) {
        unsafe { SetEvent(self.raw()).unwrap() };
    }
    fn reset(&self) {
        unsafe { ResetEvent(self.raw()).unwrap() };
    }
}

const SHORT: Option<Duration> = Some(Duration::from_millis(200));

fn tagged(key: usize) -> Completion {
    Completion {
        key,
        overlapped: 0xABC0 + key,
        bytes: 7,
        status: 0,
    }
}

#[test]
fn posts_come_back_in_bulk() {
    let port = CompletionPort::new(0).unwrap();
    for k in 1..=3 {
        port.post(tagged(k)).unwrap();
    }
    let mut got = Vec::new();
    assert_eq!(port.dequeue_many(&mut got, 8, SHORT).unwrap(), 3);
    assert_eq!(got.iter().map(|c| c.key).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(got[0], tagged(1));
    assert_eq!(port.dequeue(SHORT).unwrap(), None);
}

/// An event's signal arrives as a completion carrying what the packet
/// was armed with -- once, until armed again.
#[test]
fn a_signalled_event_completes_once_per_arming() {
    let port = CompletionPort::new(0).unwrap();
    let event = Event::new();
    let packet = WaitPacket::new().unwrap();
    let armed = Completion { status: 0x42, ..tagged(9) };
    assert!(!packet.associate(&port, event.0.as_handle(), armed).unwrap());
    assert_eq!(port.dequeue(SHORT).unwrap(), None, "fired before the signal");
    event.set();
    assert_eq!(port.dequeue(SHORT).unwrap(), Some(armed));
    // One-shot: still signalled, but nothing more until re-armed.
    assert_eq!(port.dequeue(SHORT).unwrap(), None);
    // Re-armed on a signalled event: reported, and queued at once.
    assert!(packet.associate(&port, event.0.as_handle(), armed).unwrap());
    assert_eq!(port.dequeue(SHORT).unwrap(), Some(armed));
    event.reset();
}

#[test]
fn cancelling_reports_where_the_wait_was() {
    let port = CompletionPort::new(0).unwrap();
    let event = Event::new();
    let packet = WaitPacket::new().unwrap();

    packet.associate(&port, event.0.as_handle(), tagged(1)).unwrap();
    let before = packet.cancel(true).unwrap();
    event.set();
    assert_eq!(port.dequeue(SHORT).unwrap(), None, "a cancelled wait fired");
    event.reset();

    packet.associate(&port, event.0.as_handle(), tagged(2)).unwrap();
    event.set();
    std::thread::sleep(Duration::from_millis(50));
    let removed = packet.cancel(true).unwrap();
    assert_eq!(port.dequeue(SHORT).unwrap(), None, "the removed completion stayed queued");
    event.reset();

    packet.associate(&port, event.0.as_handle(), tagged(3)).unwrap();
    event.set();
    std::thread::sleep(Duration::from_millis(50));
    let kept = packet.cancel(false).unwrap();
    assert_eq!(port.dequeue(SHORT).unwrap().map(|c| c.key), Some(3));
    eprintln!("cancel outcomes: before={before:?} removed={removed:?} kept={kept:?}");
    assert_eq!(before, CancelOutcome::Cancelled);
    assert_eq!(removed, CancelOutcome::Cancelled);
    assert_eq!(kept, CancelOutcome::AlreadyDelivered);
}

/// The wait holds the object, so the caller's handle may close.
#[test]
fn the_target_handle_may_close_after_arming() {
    let port = CompletionPort::new(0).unwrap();
    let event = Event::new();
    let duplicate = event.0.try_clone().unwrap();
    let packet = WaitPacket::new().unwrap();
    packet.associate(&port, duplicate.as_handle(), tagged(5)).unwrap();
    drop(duplicate);
    event.set();
    assert_eq!(port.dequeue(SHORT).unwrap().map(|c| c.key), Some(5));
}

/// A process's exit is a signal like any other.
#[test]
fn a_process_exit_completes() {
    let port = CompletionPort::new(0).unwrap();
    let child = std::process::Command::new("cmd")
        .args(["/c", "exit", "0"])
        .spawn()
        .unwrap();
    let packet = WaitPacket::new().unwrap();
    packet.associate(&port, child.as_handle(), tagged(11)).unwrap();
    let got = port.dequeue(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(got.map(|c| c.key), Some(11));
}

/// Waits and I/O share one port: a posted completion and a signal both
/// arrive through the same dequeue.
#[test]
fn signals_and_posts_share_a_port() {
    let port = CompletionPort::new(0).unwrap();
    let event = Event::new();
    let packet = WaitPacket::new().unwrap();
    packet.associate(&port, event.0.as_handle(), tagged(1)).unwrap();
    port.post(tagged(2)).unwrap();
    event.set();
    let mut got = Vec::new();
    while got.len() < 2 && port.dequeue_many(&mut got, 4, SHORT).unwrap() > 0 {}
    let mut keys: Vec<_> = got.iter().map(|c| c.key).collect();
    keys.sort();
    assert_eq!(keys, [1, 2]);
}

/// A process's arrival and departure, and the job emptying, arrive as
/// completions on the job's port.
#[test]
fn a_job_reports_its_processes() {
    let port = CompletionPort::new(0).unwrap();
    let job = Job::new().unwrap();
    job.notify(&port, 77).unwrap();
    // Long enough to assign before it exits.
    let mut child = std::process::Command::new("cmd")
        .args(["/c", "ping", "-n", "2", "127.0.0.1", ">nul"])
        .spawn()
        .unwrap();
    job.assign(child.as_handle()).unwrap();
    let pid = child.id();
    let mut seen = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !seen.contains(&JobMessage::ActiveProcessZero) && std::time::Instant::now() < deadline {
        if let Some(c) = port.dequeue(Some(Duration::from_millis(500))).unwrap() {
            assert_eq!(c.key, 77);
            seen.push(JobMessage::from_completion(&c));
        }
    }
    child.wait().unwrap();
    assert!(seen.contains(&JobMessage::NewProcess(pid)), "{seen:?}");
    assert!(seen.contains(&JobMessage::ExitProcess(pid)), "{seen:?}");
    assert!(seen.contains(&JobMessage::ActiveProcessZero), "{seen:?}");
}

/// With kill-on-close, dropping the job ends what it holds.
#[test]
fn dropping_a_kill_on_close_job_ends_its_processes() {
    let job = Job::new().unwrap();
    job.kill_on_close().unwrap();
    let mut child = std::process::Command::new("cmd")
        .args(["/c", "ping", "-n", "30", "127.0.0.1", ">nul"])
        .spawn()
        .unwrap();
    job.assign(child.as_handle()).unwrap();
    drop(job);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut exited = None;
    while exited.is_none() && std::time::Instant::now() < deadline {
        exited = child.try_wait().unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(exited.is_some(), "the process outlived its job");
}
