//! The `DispatcherQueue` lifecycle with no runtime on top: create a queue
//! controller on this thread, run its event loop until a queued item asks it
//! to exit, shut it down, and do it all again on the same thread.
//! `DispatcherRuntime::run` drives exactly this sequence. Needs the Windows
//! App SDK 2.x runtime installed.
//!
//! ```text
//! cargo run --example dq_controller
//! ```

use winui3::bootstrap::{PackageDependency, WindowsAppSDKVersion};
use winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueController, DispatcherQueueHandler};

fn run_once(greeting: &'static str) {
    let controller = DispatcherQueueController::CreateOnCurrentThread().unwrap();
    let queue = controller.DispatcherQueue().unwrap();
    queue.EnsureSystemDispatcherQueue().unwrap();
    queue
        .TryEnqueue(&DispatcherQueueHandler::new(move || {
            println!("{greeting}");
            DispatcherQueue::GetForCurrentThread()?.EnqueueEventLoopExit()
        }))
        .unwrap();
    queue.RunEventLoop().unwrap();
    controller.ShutdownQueue().unwrap();
}

fn main() {
    let _sdk = PackageDependency::initialize_version(WindowsAppSDKVersion::V2).unwrap();
    run_once("first controller: event loop ran");
    run_once("second controller on the same thread: event loop ran");
}
