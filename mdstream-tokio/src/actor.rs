use mdstream::{MdStream, Update};
use tokio::sync::mpsc;

use crate::{CoalesceOptions, CoalescingReceiver};

/// Spawn a task that owns `MdStream` and emits owned `Update`s.
///
/// This is useful when your consumer cannot keep `MdStream` on the UI thread, or when you want to
/// isolate parsing work from rendering.
pub fn spawn_mdstream_actor(
    mut stream: MdStream,
    rx: mpsc::Receiver<String>,
    opts: CoalesceOptions,
) -> mpsc::Receiver<Update> {
    let (tx_out, rx_out) = mpsc::channel::<Update>(64);

    tokio::spawn(async move {
        let mut rx = CoalescingReceiver::new(rx, opts);
        while let Some(chunk) = rx.recv().await {
            let u = stream.append(&chunk);
            if tx_out.send(u).await.is_err() {
                return;
            }
        }
        let u = stream.finalize();
        let _ = tx_out.send(u).await;
    });

    rx_out
}
