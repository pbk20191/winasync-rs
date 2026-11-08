mod tp;
mod comio;
mod dq;
mod async_util;

use windows::System::Threading::{ThreadPool, WorkItemHandler};
use windows::Win32::System::WinRT::ICoreWindowInterop;
use windows_future::IAsyncAction;
use windows_threading::Pool;
use winui3::bootstrap::{PackageDependency, WindowsAppSDKVersion};
use winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler};

/// Demo of the TP_* based async runtime in `src/tp/`.
fn tp_demo() {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::time::Duration;

    let rt = tp::Runtime::new();

    // Plain closure submission at an explicit priority.
    rt.submit(|| println!("[tp] hello from a pool thread"));

    // Scoped submission on a separate low-level pool (join barrier via the
    // cleanup group), borrowing a local.
    let mut squares = [0u64; 8];
    tp::Pool::with_scope(|scope| {
        for (i, slot) in squares.iter_mut().enumerate() {
            scope.submit(move || *slot = (i as u64) * (i as u64));
        }
    });
    println!("[tp] squares computed in scope: {squares:?}");

    let rt2 = rt.clone();
    let out = tp::block_on(async {
        // spawn -> JoinHandle
        let answer = rt.spawn(async { 21 * 2 }).await.unwrap();

        // TP_TIMER based sleep + timeout
        tp::sleep(&rt2, Duration::from_millis(30)).await;
        let timed_out = tp::timeout(&rt2, Duration::from_millis(20), tp::sleep(&rt2, Duration::from_secs(5)))
            .await
            .is_err();

        // Async file IO (TP_IO + OVERLAPPED)
        let path = std::env::temp_dir().join("tp_demo.bin");
        let path = path.to_string_lossy().into_owned();
        let file = tp::File::create(&rt2, &path).unwrap();
        let (w, _) = file.write_at(b"threadpool async io".to_vec(), 0).await;
        let (r, buf) = file.read_at(vec![0u8; 19], 0).await;
        assert_eq!((w.unwrap(), r.unwrap()), (19, 19));
        drop(file);
        let _ = std::fs::remove_file(&path);
        let file_text = String::from_utf8(buf).unwrap();

        // Async TCP echo (AcceptEx / ConnectEx / WSARecv / WSASend)
        let listener =
            tp::TcpListener::bind(&rt2, SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = rt2.spawn(async move {
            let s = listener.accept().await.unwrap();
            let (n, b) = s.read(vec![0u8; 64]).await;
            let n = n.unwrap();
            s.write(b[..n].to_vec()).await.0.unwrap();
        });
        let client = tp::TcpStream::connect(&rt2, addr).await.unwrap();
        client.write(b"ping".to_vec()).await.0.unwrap();
        let (n, b) = client.read(vec![0u8; 64]).await;
        server.await.unwrap();
        let echo = String::from_utf8(b[..n.unwrap()].to_vec()).unwrap();

        (answer, timed_out, file_text, echo)
    });
    println!("[tp] answer={} timeout_works={} file={:?} echo={:?}", out.0, out.1, out.2, out.3);
    drop(rt); // waits for outstanding callbacks via CloseThreadpoolCleanupGroupMembers
    println!("[tp] runtime torn down cleanly");
}

/// Demo of the `dq` DispatcherQueue-based async runtime in `src/dq/`,
/// showcasing it composing with `comio`'s named-pipe transport.
///
/// Must run after the Windows App SDK bootstrap
/// (`PackageDependency::initialize_version`) has been initialized:
/// `DispatcherRuntime::run` creates a real WinRT `DispatcherQueue`, which
/// requires that bootstrap to be in place first (see the ordering note on
/// `main`).
fn dq_demo() {
    use dq::DispatcherRuntime;
    use std::time::Duration;

    let name = format!(r"\\.\pipe\dq_demo_{}", std::process::id());
    let sname = name.clone();
    // Self-contained echo server on a plain thread, driven by `tp::block_on`
    // (comio is runtime-agnostic) — mirrors `dq::integration::dq_named_pipe_echo`.
    let server = std::thread::spawn(move || {
        use comio::buffer::new_buffer;
        use comio::NamedPipeListener;
        use std::future::IntoFuture;
        use tp::block_on;
        use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
        use windows_core::Interface;

        let l = NamedPipeListener::create(&sname);
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
        use comio::NamedPipeClient;
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
        let c = NamedPipeClient::connect(&name).await.unwrap();
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

fn main() {
    // `DispatcherRuntime::run` (used by `dq_demo`, and by the existing WinUI
    // section below) creates a real WinRT `DispatcherQueue`, which requires
    // the Windows App SDK bootstrap to already be initialized. Moved to the
    // very top of `main` (it used to run between `tp_demo()` and the WinUI
    // section) so both `tp_demo()` and `dq_demo()` — the latter needs it, the
    // former doesn't but is unaffected — run under it uniformly, and the
    // existing WinUI code path below keeps working exactly as before.
    let sdk = PackageDependency::initialize_version(WindowsAppSDKVersion::V2).unwrap();

    tp_demo();
    dq_demo();

  //  initialize().unwrap();
    //
    // IAsyncAction

    // windows::System::Threading::Core::PreallocatedWorkItem
    // let tp = windows::System::Threading::ThreadPool;
    // tp.RunAsync(handler);
    let mut controller = winui3::Microsoft::UI::Dispatching::DispatcherQueueController::CreateOnCurrentThread().unwrap();
    let mut dispatch_queue = controller.DispatcherQueue().unwrap();
    dispatch_queue.EnsureSystemDispatcherQueue().unwrap();
    // ICoreWindowInterop
    // winui3::Microsoft::UI::Xaml::FrameworkElement

    // CoreW
    // let t = IAsyncAction::
    {
        let server = windows::Networking::Sockets::StreamSocketListener::new().unwrap();
        server.Information().unwrap();
        let queue = DispatcherQueue::GetForCurrentThread().unwrap();
        queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
            // server.
            return Ok(());
        })).unwrap();
        windows::Web::Http::HttpClient::new().unwrap().Close().unwrap();
        // server.TransferOwnership()
        
        // server.BindServiceNameWithProtectionLevelAndAdapterAsync()
        server.Close().unwrap();
    }
    windows::Networking::Sockets::DatagramSocket::new;
   // windows::Storage::FileIO::AppendLinesAsync;
windows::Networking::Sockets::StreamSocketListener::new;
    ThreadPool::RunAsync(&WorkItemHandler::new(move |x| {

        return Ok(());
    })).unwrap();
    Pool::new().scope(|pool| {
   //   pool.0;

          ()
    });
 //   winui3::Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread().unwrap();
    dispatch_queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
        println!("Hello, world!");
        let dispatch_queue = winui3::Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread().unwrap();
        dispatch_queue.EnqueueEventLoopExit().unwrap();

        return Ok(());
    })).unwrap();
    dispatch_queue.RunEventLoop().unwrap();
    controller.ShutdownQueue().unwrap();
    drop(controller);
    drop(dispatch_queue);
    controller = winui3::Microsoft::UI::Dispatching::DispatcherQueueController::CreateOnCurrentThread().unwrap();

    dispatch_queue = controller.DispatcherQueue().unwrap();
    println!("new controller created");
    dispatch_queue.EnsureSystemDispatcherQueue().unwrap();
    dispatch_queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
        println!("Hello, world! next");
        let dispatch_queue = winui3::Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread().unwrap();
        dispatch_queue.EnqueueEventLoopExit().unwrap();
        return Ok(());
    })).unwrap();
    dispatch_queue.RunEventLoop().unwrap();
    drop(sdk);
    println!("Hello2, world!");

}
