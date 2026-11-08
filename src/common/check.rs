//! `check`: the shared "only fails on resource exhaustion → panic" policy used
//! by the thread-pool / overlapped-IO calls in `tp` and `comio`.

/// Unwraps a `windows_core::Result` that, when used correctly, only fails on
/// memory/resource exhaustion; maps such failures to a panic.
pub fn check<T>(result: windows_core::Result<T>) -> T {
    result.expect("overlapped/threadpool allocation failed")
}
