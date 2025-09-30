use async_std::channel::{Sender as AsyncSender, Receiver as AsyncReceiver, bounded};
use async_std::future::timeout as async_timeout;
use async_std::task;
use crate::rawconn::{RawConn, open_raw_connection_with_config, RawConnConfig};
use crate::tosql::SqlParam;
use crate::LpsqlError;
use std::sync::mpsc::{channel as std_channel, Sender as StdSender, Receiver as StdReceiver};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;
use log::debug;

/// Async wrapper around the synchronous RawConn.
/// Spawns a dedicated thread that owns the RawConn and performs all blocking
/// libpq operations there. Notifications are forwarded to async consumers via
/// an async_std channel.
pub struct RawConnAsync {
    cmd_tx: StdSender<Command>,
    notify_rx: AsyncReceiver<(String, String)>,
    handle: Option<thread::JoinHandle<()>>,
    /// Flag used to signal the background thread to stop. Allows Drop to request
    /// shutdown without blocking.
    running: Arc<AtomicBool>,
}

enum Command {
    Exec(String, Vec<SqlParam>, StdSender<Result<u64, LpsqlError>>),
    Listen(String, StdSender<Result<(), LpsqlError>>),
    Unlisten(String, StdSender<Result<(), LpsqlError>>),
    Close(StdSender<()>),
}

impl RawConnAsync {
    /// Open an async wrapper around a new RawConn with optional config.
    pub fn open_with_config(cfg: Option<RawConnConfig>) -> Result<Self, LpsqlError> {
        // Create the RawConn in this thread first so we can return error if connection fails.
        let mut rc = open_raw_connection_with_config(cfg)?;
        // Create channels
        let (cmd_tx, cmd_rx) = std_channel::<Command>();
        let (notify_tx, notify_rx) = bounded::<(String, String)>(100);

        // Running flag is used so Drop can request shutdown without blocking.
        let running = Arc::new(AtomicBool::new(true));
        let running_bg = running.clone();

        // Spawn the dedicated thread that owns the RawConn.
        let handle = thread::spawn(move || {
            debug!("RawConnAsync: background thread started");
            // Background loop: process commands, then wait for notifications with a short timeout.
            while running_bg.load(Ordering::SeqCst) {
                // Drain commands first so control operations are responsive.
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        Command::Exec(sql, params, resp_tx) => {
                            let res = rc.exec(&sql, params);
                            let _ = resp_tx.send(res);
                        }
                        Command::Listen(ch, resp_tx) => {
                            let res = rc.listen(&ch);
                            let _ = resp_tx.send(res);
                        }
                        Command::Unlisten(ch, resp_tx) => {
                            let res = rc.unlisten(&ch);
                            let _ = resp_tx.send(res);
                        }
                        Command::Close(resp_tx) => {
                            // Received explicit close request: stop running and exit thread.
                            running_bg.store(false, Ordering::SeqCst);
                            rc.close();
                            let _ = resp_tx.send(());
                            debug!("RawConnAsync: background thread exiting on Close");
                            return;
                        }
                    }
                }

                // Wait for notification with a short timeout so we periodically check for commands.
                match rc.wait_for_notify(Some(Duration::from_millis(500))) {
                    Ok(Some((ch, payload))) => {
                        // Best-effort send into async channel (ignore if queue full).
                        let _ = notify_tx.try_send((ch, payload));
                    }
                    Ok(None) => {
                        // timeout, no notification - loop again
                    }
                    Err(e) => {
                        // Log transient errors and continue; reconnect logic lives in RawConn.
                        debug!("RawConnAsync background: wait_for_notify error: {:?}", e);
                    }
                }
            }

            // If we exit loop due to running flag = false, perform a clean close.
            debug!("RawConnAsync: background thread shutting down (running flag cleared)");
            // Best-effort close (rc.close is idempotent)
            let _ = std::panic::catch_unwind(|| rc.close());
        });

        Ok(Self {
            cmd_tx,
            notify_rx,
            handle: Some(handle),
            running,
        })
    }

    /// Convenience to open with default config.
    pub fn open() -> Result<Self, LpsqlError> {
        Self::open_with_config(None)
    }

    /// Async listen: send LISTEN command to background thread.
    pub async fn listen(&self, channel: &str) -> Result<(), LpsqlError> {
        let (tx, rx): (StdSender<Result<(), LpsqlError>>, StdReceiver<Result<(), LpsqlError>>) = std_channel();
        self.cmd_tx.send(Command::Listen(channel.to_string(), tx))
            .map_err(|_| LpsqlError::UnexpectedError("command channel closed".to_string()))?;
        // Block waiting for response using spawn_blocking
        task::spawn_blocking(move || rx.recv().unwrap()).await
    }

    /// Async unlisten
    pub async fn unlisten(&self, channel: &str) -> Result<(), LpsqlError> {
        let (tx, rx): (StdSender<Result<(), LpsqlError>>, StdReceiver<Result<(), LpsqlError>>) = std_channel();
        self.cmd_tx.send(Command::Unlisten(channel.to_string(), tx))
            .map_err(|_| LpsqlError::UnexpectedError("command channel closed".to_string()))?;
        task::spawn_blocking(move || rx.recv().unwrap()).await
    }

    /// Async exec with parameters.
    pub async fn exec(&self, sql: &str, params: Vec<SqlParam>) -> Result<u64, LpsqlError> {
        let (tx, rx): (StdSender<Result<u64, LpsqlError>>, StdReceiver<Result<u64, LpsqlError>>) = std_channel();
        self.cmd_tx.send(Command::Exec(sql.to_string(), params, tx))
            .map_err(|_| LpsqlError::UnexpectedError("command channel closed".to_string()))?;
        task::spawn_blocking(move || rx.recv().unwrap()).await
    }

    /// Try to retrieve a pending notification without waiting.
    pub async fn poll_notify(&self) -> Option<(String, String)> {
        match self.notify_rx.try_recv() {
            Ok(v) => Some(v),
            Err(_) => None,
        }
    }

    /// Wait for a notification with optional timeout.
    pub async fn wait_for_notify(&self, timeout: Option<Duration>) -> Result<Option<(String, String)>, LpsqlError> {
        if let Some(d) = timeout {
            match async_timeout(d, self.notify_rx.recv()).await {
                Ok(Ok(v)) => Ok(Some(v)),
                Ok(Err(_)) => Ok(None),
                Err(_) => Ok(None), // timeout
            }
        } else {
            match self.notify_rx.recv().await {
                Ok(v) => Ok(Some(v)),
                Err(_) => Ok(None),
            }
        }
    }

    /// Close the async wrapper and join the background thread.
    pub async fn close(mut self) {
        // Flip running to false so background thread knows we're shutting down.
        self.running.store(false, Ordering::SeqCst);

        let (tx, rx) = std_channel::<()>();
        let _ = self.cmd_tx.send(Command::Close(tx));
        // Wait for the close ack in a blocking way, but with a timeout to avoid hanging forever.
        let _ = task::spawn_blocking(move || {
            // We use recv_timeout so Close doesn't block indefinitely.
            let _ = rx.recv_timeout(Duration::from_secs(2));
        }).await;
        if let Some(handle) = self.handle.take() {
            // Best-effort join. If the thread doesn't exit, we avoid blocking forever in async context.
            let _ = handle.join();
        }
    }
}

impl Drop for RawConnAsync {
    fn drop(&mut self) {
        // Signal running = false so background thread can exit promptly.
        self.running.store(false, Ordering::SeqCst);

        // Best-effort: attempt to send Close in a background thread so Drop doesn't block.
        // If the channel is closed, ignore the error.
        let tx = self.cmd_tx.clone();
        std::thread::spawn(move || {
            let resp = std_channel::<()>();
            let _ = tx.send(Command::Close(resp.0));
        });
        // Note: background thread may still be running for a short while; callers should call close() explicitly.
    }
}
