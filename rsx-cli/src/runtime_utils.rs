use crate::output::Failure;
use std::future::Future;
use tokio::runtime::Builder;

/// Runs `future` to completion on a fresh current-thread runtime.
pub fn block_on<T>(future: impl Future<Output = Result<T, Failure>>) -> Result<T, Failure> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Failure::runtime(format!("failed to start the runtime: {error}")))?;
    runtime.block_on(future)
}
