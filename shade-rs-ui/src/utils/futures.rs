use std::future::Future;

use leptos::spawn_local;

pub fn spawn_local_and_handle_error<
    F: Future<Output = Result<(), E>> + 'static,
    E: std::error::Error,
>(
    fut: F,
) {
    spawn_local(async move {
        if let Err(error) = fut.await {
            let mut error: &dyn std::error::Error = &error;

            tracing::error!(%error);

            while let Some(source) = error.source() {
                tracing::error!(%source);
                error = source;
            }
        }
    });
}
