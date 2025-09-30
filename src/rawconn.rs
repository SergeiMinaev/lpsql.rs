use crate::conf::Conf;
use crate::tosql::{ToSql, SqlParam};
use crate::LpsqlError;
use pq_sys::*;
use pq_sys::ConnStatusType::CONNECTION_OK;
use pq_sys::ExecStatusType::{PGRES_COMMAND_OK, PGRES_TUPLES_OK, PGRES_BAD_RESPONSE, PGRES_FATAL_ERROR};
use std::ffi::{CString, CStr};
use std::ptr;
use std::time::Duration;
use std::collections::{VecDeque, HashSet};
use std::os::raw::{c_int, c_void};
use log::debug;

/// Minimal synchronous RawConn implementation (Step A prototype).
///
/// Notes / constraints:
/// - This is a minimal, blocking implementation intended as the prototype described
///   in the listen-notify plan. It keeps a single PGconn and exposes blocking APIs.
/// - exec supports parameters via tosql::SqlParam; both zero-param and parameterized
///   queries are supported using PQexec and PQexecParams.
/// - All libpq calls are made from the current thread. If you call these APIs from
///   an async runtime, run them on a dedicated thread or via spawn_blocking.
///
/// Important: this implementation follows the project's existing pattern of wrapping
/// PQconnectdb result in a Box<PGconn> (same as LpsqlConn::setup). Memory handling
/// mirrors existing code paths in the repo.
pub struct RawConn {
    conf: Conf,
    conn: Box<PGconn>,
    listened: HashSet<String>,
    notify_queue: VecDeque<(String, String)>,
    reconnect_backoff_ms: u64,
    max_reconnect_backoff_ms: u64,
    // cache of prepared statement hashes to avoid repeated PQprepare calls
    prepared_statements: HashSet<u64>,
}

fn validate_channel_name(name: &str) -> bool {
    // PostgreSQL identifier rules (simplified):
    // - non-empty, up to 63 bytes
    // - start with a letter (A-Z, a-z) or underscore
    // - subsequent chars may be letters, digits, or underscore
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    let bytes = name.as_bytes();
    let first = bytes[0];
    let valid_first = (b'A'..=b'Z').contains(&first) || (b'a'..=b'z').contains(&first) || first == b'_';
    if !valid_first {
        return false;
    }
    for &b in &bytes[1..] {
        if !((b'A'..=b'Z').contains(&b) ||
             (b'a'..=b'z').contains(&b) ||
             (b'0'..=b'9').contains(&b) ||
             b == b'_') {
            return false;
        }
    }
    true
}

fn hash_query(query: &str) -> u64 {
    use std::hash::{Hash, Hasher, DefaultHasher};
    let mut hasher = DefaultHasher::new();
    query.hash(&mut hasher);
    hasher.finish()
}

/// Configuration for RawConn reconnection/backoff behaviour.
pub struct RawConnConfig {
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for RawConnConfig {
    fn default() -> Self {
        Self {
            initial_backoff_ms: 50,
            max_backoff_ms: 5000,
        }
    }
}

/// Open a new synchronous RawConn using the default config file "lpsql.toml".
/// This keeps the backwards-compatible `open_raw_connection()` but introduces a
/// more flexible `open_raw_connection_with_config()` that accepts reconnect params.
pub fn open_raw_connection() -> Result<RawConn, LpsqlError> {
    open_raw_connection_with_config(None)
}

/// Open a new RawConn with optional RawConnConfig to control reconnect/backoff.
pub fn open_raw_connection_with_config(cfg: Option<RawConnConfig>) -> Result<RawConn, LpsqlError> {
    let conf = Conf::new("lpsql.toml");
    let cfg = cfg.unwrap_or_default();
    let conninfo = CString::new(format!(
        "dbname={} user={} password={}", conf.dbname, conf.user, conf.password
    )).map_err(|e| LpsqlError::CStringError(format!("conninfo: {}", e)))?;

    unsafe {
        let conn_ptr = PQconnectdb(conninfo.as_ptr());
        if conn_ptr.is_null() {
            return Err(LpsqlError::ConnectionFailed)
        }
        let conn_box = Box::from_raw(conn_ptr);
        debug!("RawConn: connected to database");
        Ok(RawConn {
            conf,
            conn: conn_box,
            listened: HashSet::new(),
            notify_queue: VecDeque::new(),
            reconnect_backoff_ms: cfg.initial_backoff_ms,
            max_reconnect_backoff_ms: cfg.max_backoff_ms,
            prepared_statements: HashSet::new(),
        })
    }
}

impl RawConn {
    fn conn_ptr(&mut self) -> *mut PGconn {
        (&mut *self.conn) as *mut PGconn
    }

    /// Execute a SQL statement. Supports optional parameters via SqlParam.
    /// Returns number of affected rows as u64.
    pub fn exec(&mut self, sql: &str, params: Vec<SqlParam>) -> Result<u64, LpsqlError> {
        let csql = CString::new(sql).map_err(|e| LpsqlError::CStringError(e.to_string()))?;
        unsafe {
            let conn_ptr = self.conn_ptr();
            if PQstatus(conn_ptr) != CONNECTION_OK {
                debug!("RawConn.exec: connection not OK, attempting reconnect");
                self.reconnect()?;
            }
            let conn_ptr = self.conn_ptr();
            if PQstatus(conn_ptr) != CONNECTION_OK {
                return Err(LpsqlError::ConnectionFailed)
            }

            // If no params, use simple PQexec path.
            if params.is_empty() {
                let res = PQexec(conn_ptr, csql.as_ptr());
                if res.is_null() {
                    return Err(LpsqlError::UnexpectedError("PQexec returned null".to_string()))
                }
                let status = PQresultStatus(res);
                if status == PGRES_COMMAND_OK || status == PGRES_TUPLES_OK {
                    let tuples_ptr = PQcmdTuples(res);
                    let rows_str = if tuples_ptr.is_null() {
                        "0".to_string()
                    } else {
                        CStr::from_ptr(tuples_ptr).to_string_lossy().into_owned()
                    };
                    PQclear(res);
                    let rows = rows_str.parse::<u64>().unwrap_or(0);
                    Ok(rows)
                } else {
                    let err_msg = CStr::from_ptr(PQresultErrorMessage(res))
                        .to_string_lossy().into_owned();
                    let status = PQresultStatus(res);
                    debug!("RawConn.exec (no-params) failed: {} (status: {:?})", err_msg, status);
                    PQclear(res);
                    if status == PGRES_FATAL_ERROR {
                        return Err(LpsqlError::FatalError(err_msg))
                    } else {
                        return Err(LpsqlError::ExecuteFailed(err_msg))
                    }
                }
            } else {
                // Parameterized query: prepare once (cached) + PQexecPrepared to reuse server-side prepare.
                let n_params = params.len() as i32;
                let stmt_hash = hash_query(sql);
                let stmt_name = CString::new(format!("stmt_{}", stmt_hash)).map_err(|e| LpsqlError::CStringError(e.to_string()))?;
                let stmt = CString::new(sql).map_err(|e| LpsqlError::CStringError(e.to_string()))?;

                // Prepare the statement only if we haven't prepared it before.
                if !self.prepared_statements.contains(&stmt_hash) {
                    let prepare_res = PQprepare(
                        conn_ptr,
                        stmt_name.as_ptr(),
                        stmt.as_ptr(),
                        n_params,
                        ptr::null()
                    );

                    let prep_status = PQresultStatus(prepare_res);
                    match prep_status {
                        PGRES_COMMAND_OK => {
                            PQclear(prepare_res);
                            self.prepared_statements.insert(stmt_hash);
                        },
                        PGRES_BAD_RESPONSE => {
                            let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
                                .to_string_lossy().into_owned();
                            PQclear(prepare_res);
                            return Err(LpsqlError::BadResponse(err_msg))
                        },
                        PGRES_FATAL_ERROR => {
                            let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
                                .to_string_lossy().into_owned();
                            PQclear(prepare_res);
                            if !err_msg.contains("already exists") {
                                debug!("RawConn.exec: PQprepare fatal error: {}", err_msg);
                                return Err(LpsqlError::PrepareFailed(err_msg))
                            } else {
                                // server-side statement already exists; consider it prepared
                                self.prepared_statements.insert(stmt_hash);
                            }
                        },
                        _ => {
                            let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
                                .to_string_lossy().into_owned();
                            PQclear(prepare_res);
                            return Err(LpsqlError::UnexpectedError(err_msg))
                        }
                    }
                }

                let p_vecc_ptr: Vec<_> = params.iter().map(|arg| arg.as_ptr()).collect();
                let res = PQexecPrepared(
                    conn_ptr,
                    stmt_name.as_ptr(),
                    n_params,
                    p_vecc_ptr.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    0
                );

                if res.is_null() {
                    return Err(LpsqlError::UnexpectedError("PQexecPrepared returned null".to_string()))
                }
                let status = PQresultStatus(res);
                if status == PGRES_COMMAND_OK || status == PGRES_TUPLES_OK {
                    let tuples_ptr = PQcmdTuples(res);
                    let rows_str = if tuples_ptr.is_null() {
                        "0".to_string()
                    } else {
                        CStr::from_ptr(tuples_ptr).to_string_lossy().into_owned()
                    };
                    PQclear(res);
                    let rows = rows_str.parse::<u64>().unwrap_or(0);
                    Ok(rows)
                } else {
                    let err_msg = CStr::from_ptr(PQresultErrorMessage(res))
                        .to_string_lossy().into_owned();
                    let status = PQresultStatus(res);
                    debug!("RawConn.exec (with-params) failed: {} (status: {:?})", err_msg, status);
                    PQclear(res);
                    if status == PGRES_FATAL_ERROR {
                        return Err(LpsqlError::FatalError(err_msg))
                    } else {
                        return Err(LpsqlError::ExecuteFailed(err_msg))
                    }
                }
            }
        }
    }

    /// Attempt to reconnect with exponential backoff and re-issue LISTENs on success.
    fn reconnect(&mut self) -> Result<(), LpsqlError> {
        debug!("RawConn.reconnect: attempting to reconnect");
        let mut backoff = self.reconnect_backoff_ms;
        loop {
            std::thread::sleep(Duration::from_millis(backoff));
            let conninfo = CString::new(format!(
                "dbname={} user={} password={}", self.conf.dbname, self.conf.user, self.conf.password
            )).map_err(|e| LpsqlError::CStringError(format!("conninfo: {}", e)))?;
            unsafe {
                let new_ptr = PQconnectdb(conninfo.as_ptr());
                if !new_ptr.is_null() && PQstatus(new_ptr) == CONNECTION_OK {
                    // Build Box from new_ptr
                    let new_box = Box::from_raw(new_ptr);
                    // Replace old connection with new_box and extract old_box
                    let old_box = std::mem::replace(&mut self.conn, new_box);
                    // Prevent double-drop: take raw pointer from old_box without dropping it,
                    // then call PQfinish on that raw pointer (PQfinish frees libpq memory).
                    let old_ptr = Box::into_raw(old_box);
                    PQfinish(old_ptr);
                    debug!("RawConn.reconnect: reconnected");
                    // Re-issue LISTENs for tracked channels (best-effort; ignore individual errors)
                    for ch in self.listened.clone() {
                        let _ = self.exec(&format!("LISTEN {}", ch), Vec::new());
                    }
                    return Ok(());
                } else {
                    if !new_ptr.is_null() {
                        PQfinish(new_ptr);
                    }
                }
            }
            if backoff >= self.max_reconnect_backoff_ms {
                break;
            }
            backoff = std::cmp::min(backoff * 2, self.max_reconnect_backoff_ms);
            debug!("RawConn.reconnect: retrying in {} ms", backoff);
        }
        Err(LpsqlError::ConnectionFailed)
    }

    /// Issue a LISTEN <channel> command and track the channel on success.
    pub fn listen(&mut self, channel: &str) -> Result<(), LpsqlError> {
        if !validate_channel_name(channel) {
            return Err(LpsqlError::UnexpectedError("invalid channel name".to_string()))
        }

        // Use PQescapeIdentifier to safely escape the channel identifier and avoid SQL injection.
        unsafe {
            let conn_ptr = self.conn_ptr();
            let cch = CString::new(channel).map_err(|e| LpsqlError::CStringError(e.to_string()))?;
            let esc_ptr = PQescapeIdentifier(conn_ptr, cch.as_ptr(), channel.len() as libc::size_t);
            if esc_ptr.is_null() {
                return Err(LpsqlError::UnexpectedError("PQescapeIdentifier failed".to_string()))
            }
            let escaped = CStr::from_ptr(esc_ptr).to_string_lossy().into_owned();
            let sql = format!("LISTEN {}", escaped);
            // free escaped memory allocated by libpq
            PQfreemem(esc_ptr as *mut c_void);
            let _ = self.exec(&sql, Vec::new())?;
            self.listened.insert(channel.to_string());
            debug!("RawConn.listen: listening on channel '{}'", channel);
            Ok(())
        }
    }

    /// Issue UNLISTEN <channel> and remove it from tracked channels on success.
    pub fn unlisten(&mut self, channel: &str) -> Result<(), LpsqlError> {
        if !validate_channel_name(channel) {
            return Err(LpsqlError::UnexpectedError("invalid channel name".to_string()))
        }

        unsafe {
            let conn_ptr = self.conn_ptr();
            let cch = CString::new(channel).map_err(|e| LpsqlError::CStringError(e.to_string()))?;
            let esc_ptr = PQescapeIdentifier(conn_ptr, cch.as_ptr(), channel.len() as libc::size_t);
            if esc_ptr.is_null() {
                return Err(LpsqlError::UnexpectedError("PQescapeIdentifier failed".to_string()))
            }
            let escaped = CStr::from_ptr(esc_ptr).to_string_lossy().into_owned();
            let sql = format!("UNLISTEN {}", escaped);
            PQfreemem(esc_ptr as *mut c_void);
            let _ = self.exec(&sql, Vec::new())?;
            self.listened.remove(channel);
            debug!("RawConn.unlisten: removed listen on channel '{}'", channel);
            Ok(())
        }
    }

    /// Poll libpq for pending notifications; this drains PQnotifies and returns the first
    /// notification if present. This is non-blocking in the sense that it does not wait
    /// on the socket; it calls PQconsumeInput then PQnotifies.
    pub fn poll_notify(&mut self) -> Result<Option<(String, String)>, LpsqlError> {
        unsafe {
            let conn_ptr = self.conn_ptr();
            if PQstatus(conn_ptr) != CONNECTION_OK {
                debug!("RawConn.poll_notify: connection not OK, attempting reconnect");
                self.reconnect()?;
            }
            let conn_ptr = self.conn_ptr();
            // Ask libpq to read from socket into its buffers
            let consumed = PQconsumeInput(conn_ptr);
            if consumed == 0 {
                // consumption failed
                let err_msg = CStr::from_ptr(PQerrorMessage(conn_ptr)).to_string_lossy().into_owned();
                debug!("RawConn.poll_notify: PQconsumeInput failed: {}", err_msg);
                return Err(LpsqlError::TransientError(err_msg))
            }

            let mut first: Option<(String, String)> = None;
            loop {
                let notify_ptr = PQnotifies(conn_ptr);
                if notify_ptr.is_null() {
                    break;
                }
                // SAFETY: PGnotify pointer is owned by libpq and must be freed via PQfreemem
                // after reading its contents.
                let chan = CStr::from_ptr((*notify_ptr).relname).to_string_lossy().into_owned();
                let payload = if (*notify_ptr).extra.is_null() {
                    "".to_string()
                } else {
                    CStr::from_ptr((*notify_ptr).extra).to_string_lossy().into_owned()
                };
                if first.is_none() {
                    first = Some((chan.clone(), payload.clone()));
                } else {
                    // queue additional notifications
                    self.notify_queue.push_back((chan.clone(), payload.clone()));
                }
                // Free the PGnotify memory returned by PQnotifies
                PQfreemem(notify_ptr as *mut c_void);
            }
            Ok(first)
        }
    }

    /// Blocking wait for a notification with optional timeout. Uses PQsocket + poll.
    /// timeout: None -> wait indefinitely; Some(d) -> wait up to d.
    pub fn wait_for_notify(&mut self, timeout: Option<Duration>) -> Result<Option<(String, String)>, LpsqlError> {
        // First check if we already have queued notifications
        if let Some(v) = self.notify_queue.pop_front() {
            return Ok(Some(v));
        }

        unsafe {
            let conn_ptr = self.conn_ptr();
            if PQstatus(conn_ptr) != CONNECTION_OK {
                debug!("RawConn.wait_for_notify: connection not OK, attempting reconnect");
                self.reconnect()?;
            }
            let conn_ptr = self.conn_ptr();
            let sock = PQsocket(conn_ptr);
            if sock < 0 {
                return Err(LpsqlError::ConnectionFailed)
            }

            // prepare pollfd
            let mut pfd = libc::pollfd {
                fd: sock,
                events: libc::POLLIN,
                revents: 0,
            };
            let timeout_ms = match timeout {
                Some(d) => {
                    let ms = d.as_millis();
                    if ms > c_int::MAX as u128 { c_int::MAX } else { ms as c_int }
                },
                None => -1,
            };

            let ret = libc::poll(&mut pfd as *mut libc::pollfd, 1, timeout_ms);
            if ret < 0 {
                debug!("RawConn.wait_for_notify: poll failed with ret < 0");
                return Err(LpsqlError::TransientError("poll failed".to_string()))
            } else if ret == 0 {
                debug!("RawConn.wait_for_notify: poll timed out ({} ms)", timeout_ms);
                return Ok(None) // timeout
            } else {
                // socket readable -> consume and collect notifications
                return self.poll_notify()
            }
        }
    }

    /// Close the connection. After this RawConn must not be used.
    pub fn close(self) {
        unsafe {
            let mut boxed = self.conn;
            let conn_ptr = (&mut *boxed) as *mut PGconn;
            PQfinish(conn_ptr);
            // boxed is dropped here
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use pq_sys::*;

    fn unique_channel(base: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // channel names must be alphanumeric + underscore
        format!("{}_{}", base, nanos)
    }

    fn send_notify(channel: &str, payload: &str) {
        let conf = Conf::new("lpsql.toml");
        let conninfo = CString::new(format!(
            "dbname={} user={} password={}",
            conf.dbname, conf.user, conf.password
        ))
        .unwrap();
        unsafe {
            let conn_ptr = PQconnectdb(conninfo.as_ptr());
            assert!(!conn_ptr.is_null(), "failed to connect in test sender");
            let sql = CString::new(format!("SELECT pg_notify('{}','{}')", channel, payload)).unwrap();
            let res = PQexec(conn_ptr, sql.as_ptr());
            if !res.is_null() {
                PQclear(res);
            }
            PQfinish(conn_ptr);
        }
    }

    // Helper to kill the backend for a RawConn's connection. This simulates a server-side
    // disconnect without calling PQfinish on the RawConn's Box, avoiding double-free.
    fn kill_backend(rc: &mut RawConn) {
        let pid = unsafe { PQbackendPID(rc.conn_ptr()) };
        if pid <= 0 {
            return;
        }

        let conf = Conf::new("lpsql.toml");
        let conninfo = CString::new(format!(
            "dbname={} user={} password={}",
            conf.dbname, conf.user, conf.password
        ))
        .unwrap();

        unsafe {
            let other = PQconnectdb(conninfo.as_ptr());
            assert!(!other.is_null(), "failed to connect for backend-killer");
            let sql = CString::new(format!("SELECT pg_terminate_backend({})", pid)).unwrap();
            let res = PQexec(other, sql.as_ptr());
            if !res.is_null() {
                PQclear(res);
            }
            PQfinish(other);
        }
    }

    #[test]
    fn test_notify_roundtrip() {
        let mut rc = open_raw_connection().expect("open_raw_connection failed");
        let channel = unique_channel("test_roundtrip");
        rc.listen(&channel).expect("listen failed");
        send_notify(&channel, "payload");
        let got = rc
            .wait_for_notify(Some(Duration::from_secs(2)))
            .expect("wait_for_notify failed");
        assert_eq!(got, Some((channel.clone(), "payload".to_string())));
        rc.unlisten(&channel).expect("unlisten failed");
        rc.close();
    }

    #[test]
    fn test_poll_notify() {
        let mut rc = open_raw_connection().expect("open_raw_connection failed");
        let channel = unique_channel("test_poll");
        rc.listen(&channel).expect("listen failed");
        send_notify(&channel, "p1");
        // poll immediately
        let got = rc.poll_notify().expect("poll_notify failed");
        assert_eq!(got, Some((channel.clone(), "p1".to_string())));
        rc.unlisten(&channel).expect("unlisten failed");
        rc.close();
    }

    #[test]
    fn test_wait_timeout() {
        let mut rc = open_raw_connection().expect("open_raw_connection failed");
        let channel = unique_channel("test_timeout");
        rc.listen(&channel).expect("listen failed");
        let got = rc
            .wait_for_notify(Some(Duration::from_millis(100)))
            .expect("wait_for_notify failed");
        assert_eq!(got, None);
        rc.unlisten(&channel).expect("unlisten failed");
        rc.close();
    }

    #[test]
    fn test_reconnect_and_relisten() {
        // Use a tight backoff so the test runs quickly.
        let cfg = RawConnConfig { initial_backoff_ms: 10, max_backoff_ms: 500 };
        let mut rc = open_raw_connection_with_config(Some(cfg)).expect("open_raw_connection failed");
        let channel = unique_channel("test_reconnect");
        rc.listen(&channel).expect("listen failed");

        // Simulate a backend crash for this RawConn; reconnect() should detect and recover.
        kill_backend(&mut rc);

        // Request an immediate reconnect so the test can send the notification after re-listen.
        rc.reconnect().expect("reconnect failed");

        // Give a small moment for server to drop the backend and for LISTEN to be re-issued.
        std::thread::sleep(Duration::from_millis(50));

        // Send a notify from a separate connection. The RawConn should have reconnected and re-listened.
        send_notify(&channel, "after_drop");

        let got = rc
            .wait_for_notify(Some(Duration::from_secs(5)))
            .expect("wait_for_notify failed");
        assert_eq!(got, Some((channel.clone(), "after_drop".to_string())));

        rc.unlisten(&channel).expect("unlisten failed");
        rc.close();
    }
}
