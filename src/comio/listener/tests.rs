use super::*;
use windows_core::Interface;

/// A real stream rather than a stand-in: the args object is typed now,
/// so there is no `IInspectable` to substitute anything else for.
#[test]
fn args_exposes_stream_and_endpoint() {
    let (stream, _other_end) = crate::comio::stream::tests::pipe_pair();
    let args: IConnectionReceivedArgs =
        new_args(stream.clone(), windows_core::HSTRING::from("pipe://x"), PeerIdentity::default())
            .cast()
            .unwrap();
    // Stream() returns the same object identity.
    assert_eq!(args.Stream().unwrap(), stream);
    assert_eq!(args.RemoteEndpoint().unwrap().to_string(), "pipe://x");
}
