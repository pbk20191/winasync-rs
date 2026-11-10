//! The `DispatcherQueue` runtime composing with `comio`'s named-pipe
//! transport: local tasks, a queue timer, and an echo round trip awaited on
//! the queue thread. Needs the Windows App SDK 2.x runtime installed.
//!
//! ```text
//! cargo run --example dq_named_pipe
//! ```

use winasync::{comio, dq, tp};
use winui3::bootstrap::{PackageDependency, WindowsAppSDKVersion};

fn main() {
    use dq::DispatcherRuntime;
    use std::time::Duration;

    let _sdk = PackageDependency::initialize_version(WindowsAppSDKVersion::V2).unwrap();

    let name = format!(r"\\.\pipe\dq_demo_{}", std::process::id());
    let sname = name.clone();
    // Self-contained echo server on a plain thread, driven by `tp::block_on`
    // (comio is runtime-agnostic) — mirrors `dq::integration::dq_named_pipe_echo`.
    let server = std::thread::spawn(move || {
        use comio::buffer::new_buffer;
        use comio::{NamedPipeListener, NamedPipeOptions};
        use std::future::IntoFuture;
        use tp::block_on;
        use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
        use windows_core::Interface;

        let l = NamedPipeListener::create(&sname, &NamedPipeOptions::new()).unwrap();
        let s = block_on(l.accept()).unwrap();
        let i: IInputStream = s.cast().unwrap();
        let o: IOutputStream = s.cast().unwrap();
        let rb = new_buffer(16).unwrap();
        let got: IBuffer = block_on(
            i.ReadAsync(&rb.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        block_on(o.WriteAsync(&got).unwrap().into_future()).unwrap();
    });
    std::thread::sleep(Duration::from_millis(100));

    let (spawned, slept_ms, echoed) = DispatcherRuntime::run(async move {
        use comio::buffer::{buffer_ptr, new_buffer};
        use comio::NamedPipeConnector;
        use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
        use windows_core::Interface;

        // spawn_local a couple of tasks and await them.
        let h1 = dq::spawn_local(async { 1 + 1 });
        let h2 = dq::spawn_local(async { 2 + 2 });
        let spawned = (h1.await.unwrap(), h2.await.unwrap());

        // dq::sleep on the DispatcherQueueTimer backend.
        let start = std::time::Instant::now();
        dq::sleep(Duration::from_millis(20)).await;
        let slept_ms = start.elapsed().as_millis();

        // Named-pipe echo round-trip, connecting to the server thread above.
        let c = NamedPipeConnector::connect(&name).await.unwrap();
        let o: IOutputStream = c.cast().unwrap();
        let i: IInputStream = c.cast().unwrap();

        let wb = new_buffer(4).unwrap();
        unsafe {
            buffer_ptr(&wb.clone().into())
                .unwrap()
                .copy_from_nonoverlapping(b"demo".as_ptr(), 4);
        }
        wb.SetLength(4).unwrap();
        o.WriteAsync(&wb.cast::<IBuffer>().unwrap()).unwrap().await.unwrap();

        let rb = new_buffer(4).unwrap();
        let got: IBuffer = i
            .ReadAsync(&rb.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
            .unwrap()
            .await
            .unwrap();
        let echoed = unsafe { std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 4).to_vec() };

        (spawned, slept_ms, echoed)
    });

    server.join().unwrap();
    println!(
        "[dq] spawn_local results={spawned:?} slept>={slept_ms}ms echoed={:?}",
        String::from_utf8_lossy(&echoed)
    );
    println!("[dq] runtime run() returned cleanly");
}
