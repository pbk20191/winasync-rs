mod tp;
mod comio;

use windows::System::Threading::{ThreadPool, WorkItemHandler};
use windows::Win32::System::WinRT::ICoreWindowInterop;
use windows_future::IAsyncAction;
use windows_threading::Pool;
use winui3::bootstrap::{PackageDependency, WindowsAppSDKVersion};
use winui3::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler};

/// Demo of the TP_* based async runtime in `src/tp/`.
fn tp_demo() {
    use std::net::{Ipv4Addr, SocketAddrV4};
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
        let listener = tp::TcpListener::bind(&rt2, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
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

fn main() {
    tp_demo();

    let sdk = PackageDependency::initialize_version(WindowsAppSDKVersion::V2).unwrap();
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
