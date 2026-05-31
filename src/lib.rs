use std::fmt;
use std::hash::{Hash, Hasher};
use std::hash::DefaultHasher;
use std::ffi::{CString, CStr};
use std::{str, ptr};
use std::sync::Arc;
use std::boxed::Box;
use std::time::{Instant, Duration};
use std::collections::HashSet;
use pq_sys::ConnStatusType::CONNECTION_OK;
use pq_sys::ExecStatusType::PGRES_COMMAND_OK;
use pq_sys::ExecStatusType::PGRES_TUPLES_OK;
use pq_sys::ExecStatusType::PGRES_BAD_RESPONSE;
use pq_sys::ExecStatusType::PGRES_FATAL_ERROR;
use pq_sys::*;
use crate::conf::Conf;
use crate::tosql::{ToSql, SqlParam};
use async_std::sync::Mutex;
use log::debug;
use crate::pool::ConnectionPool;

pub mod conf;
pub mod pool;
pub mod tosql;
pub mod rawconn;
pub mod rawconn_async;


#[derive(Debug)]
pub enum LpsqlError {
    ConnectionFailed,
    TransientError(String),
    Timeout,
    PrepareFailed(String),
    ExecuteFailed(String),
    BadResponse(String),
    FatalError(String),
    UnexpectedError(String),
    CStringError(String),
}
impl fmt::Display for LpsqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LpsqlError::ConnectionFailed => write!(f, "ConnectionFailed"),
            LpsqlError::TransientError(reason) => write!(f, "TransientError: {}", reason),
            LpsqlError::Timeout => write!(f, "Timeout"),
            LpsqlError::PrepareFailed(reason) => write!(f, "PrepareFailed: {}", reason),
            LpsqlError::ExecuteFailed(reason) => write!(f, "ExecuteFailed: {}", reason),
            LpsqlError::BadResponse(reason) => write!(f, "BadResponse: {}", reason),
            LpsqlError::FatalError(reason) => write!(f, "FatalError: {}", reason),
            LpsqlError::UnexpectedError(reason) => write!(f, "UnexpectedError: {}", reason),
            LpsqlError::CStringError(reason) => write!(f, "CStringError: {}", reason),
        }
    }
}


pub struct Lpsql {
	query: String,
	prms: Vec<SqlParam>,

}
impl Lpsql {
	pub fn query(query: &str) -> Self {
		Self { query: query.to_string(), prms: Vec::new() }
	}
	pub fn bind<T: ToSql>(&mut self, p: T) -> &mut Self {
		self.prms.push(p.to_sql());
		self
	}
	pub async fn exec(&mut self, pool: &ConnectionPool) -> i32 {
        let conn: LpsqlConn = pool.get_conn().await;
		let r = conn.exec(&self.query, self.prms.clone()).await.unwrap();
        pool.release_conn(conn).await;
		return r
	}
	pub async fn fetch_all(&mut self, pool: &ConnectionPool) -> Vec<String> {
        let conn: LpsqlConn = pool.get_conn().await;
		match conn.fetchall(&self.query, self.prms.clone()).await {
			Err(e) => {
				// Emit failing query to stderr (per request) and return an empty result.
				eprintln!("Error executing SQL. Error: {e}. Query: {}", self.query);
				debug!("Lpsql.fetch_all LpsqlError: {:?}", e);
				pool.release_conn(conn).await;
				Vec::new()
			},
			Ok(r) => {
				pool.release_conn(conn).await;
				r
			}
		}
	}
	pub async fn fetch_one(&mut self, pool: &ConnectionPool) -> Option<String> {
        let conn: LpsqlConn = pool.get_conn().await;
		match conn.fetch_one(&self.query, self.prms.clone()).await {
			Err(e) => {
				// Emit failing query to stderr (per request) and return None.
				eprintln!("Error executing SQL. Error: {e}. Query: {}", self.query);
				debug!("Lpsql.fetch_one LpsqlError: {:?}", e);
				pool.release_conn(conn).await;
				None
			},
			Ok(r) => {
				pool.release_conn(conn).await;
				r
			}
		}
	}
}


pub struct LpsqlConn {
    pub conf: Conf,
	pub conn: Arc<Mutex<Box<PGconn>>>,
    pub last_used: Instant,
	pub conn_timeout: Duration,
    prepared_statements: Arc<std::sync::Mutex<HashSet<u64>>>,
}

// SAFETY: LpsqlConn доступен только через Mutex, raw pointer на PGconn не утекает
unsafe impl Send for LpsqlConn {}
unsafe impl Sync for LpsqlConn {}

impl LpsqlConn {
	pub fn setup(conf: Conf, conn_timeout: Duration) -> Self {
		let conninfo = CString::new(format!(
			"dbname={} user={} password={}", conf.dbname, conf.user, conf.password
		)).unwrap();
		let conn_ptr = unsafe { PQconnectdb(conninfo.as_ptr()) };
		if conn_ptr.is_null() {
			panic!("Failed to connect to the database.");
		}
		let conn_box = unsafe { Box::from_raw(conn_ptr) };
		Self {
			conf: conf,
			conn: Arc::new(Mutex::new(conn_box)),
			last_used: Instant::now(),
			conn_timeout: conn_timeout,
			prepared_statements: Arc::new(std::sync::Mutex::new(HashSet::new())),
		}
	}
	pub async fn is_active(&self) -> bool {
		let conn_ptr = {
			let mut conn_lock = self.conn.lock().await;
			(*conn_lock).as_mut() as *mut PGconn
		};
		unsafe { PQstatus(conn_ptr) == CONNECTION_OK }
	}
    fn is_timeout_exceed(&self) -> bool {
        self.last_used.elapsed() > self.conn_timeout
    }
    fn touch(&mut self) {
        self.last_used = Instant::now();
    }
	pub async fn inner_exec(&self, query: &str, params: Vec<SqlParam>) -> Result<*mut pg_result, LpsqlError> {
		let conn_ptr = {
			let mut conn_lock = self.conn.lock().await;
			(*conn_lock).as_mut() as *mut PGconn
		};

		unsafe {
			if PQstatus(conn_ptr) != CONNECTION_OK {
				let err_msg = CStr::from_ptr(PQerrorMessage(conn_ptr)).to_string_lossy().into_owned();
				debug!("LpsqlConn.inner_exec: connection not OK: {}", err_msg);
				return Err(LpsqlError::TransientError(err_msg))
			}

			let stmt_hash = hash_query(query);
			let stmt_name = CString::new(format!("stmt_{}", stmt_hash)).unwrap();
			let stmt = CString::new(query).unwrap();
			let n_params = params.len() as i32;

			let already_prepared = self.prepared_statements.lock().unwrap().contains(&stmt_hash);
			if !already_prepared {
				let prepare_res = PQprepare(
					conn_ptr, stmt_name.as_ptr(), stmt.as_ptr(), n_params, ptr::null()
				);

				match PQresultStatus(prepare_res) {
					PGRES_COMMAND_OK => {
						PQclear(prepare_res);
						self.prepared_statements.lock().unwrap().insert(stmt_hash);
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
						if err_msg.contains("already exists") {
							self.prepared_statements.lock().unwrap().insert(stmt_hash);
						} else {
							debug!("LpsqlConn.PQprepare fatal error: {}", err_msg);
							return Err(LpsqlError::FatalError(err_msg))
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
			let res_ptr = PQexecPrepared(conn_ptr, stmt_name.as_ptr(), n_params, p_vecc_ptr.as_ptr(), ptr::null(), ptr::null(), 0);
			return Ok(res_ptr)
		}
	}
	pub async fn fetchall(&self, query: &str, params: Vec<SqlParam>) -> Result<Vec<String>, LpsqlError> {
		unsafe {
			match self.inner_exec(query, params).await {
				Err(e) => {
					eprintln!("Error executing SQL: {}", query);
					debug!("LpsqlError: {e}");
					return Err(e)
				},
				Ok(res_ptr) => {
					// Handle result
					let status = PQresultStatus(res_ptr);
					if status == PGRES_TUPLES_OK || status == PGRES_COMMAND_OK {
						let mut results = Vec::new();
						let num_rows = PQntuples(res_ptr);
						let num_cols = PQnfields(res_ptr);

						for row_idx in 0..num_rows {
							for col_idx in 0..num_cols {
								let value_ptr = PQgetvalue(res_ptr, row_idx, col_idx);
								let field_val = CStr::from_ptr(value_ptr).to_string_lossy().into_owned();
								results.push(field_val);
							}
						}

						PQclear(res_ptr);
						return Ok(results)
					} else {
						let err_msg = CStr::from_ptr(PQresultErrorMessage(res_ptr))
							.to_string_lossy().into_owned();
						let _status_str = match status {
							PGRES_TUPLES_OK => "PGRES_TUPLES_OK",
							PGRES_COMMAND_OK => "PGRES_COMMAND_OK",
							PGRES_BAD_RESPONSE => "PGRES_BAD_RESPONSE",
							_ => "Unknown status",
						};
						PQclear(res_ptr);
						debug!("Lpsql.fetchall query error: {}", err_msg);
						return Err(LpsqlError::ExecuteFailed(err_msg))
					}
				}
			}
		}
	}
	pub async fn exec(&self, query: &str, params: Vec<SqlParam>) -> Result<i32, LpsqlError> {
		unsafe {
			let res_ptr = self.inner_exec(query, params).await.unwrap();
			let status = PQresultStatus(res_ptr);
			if status == PGRES_COMMAND_OK || status == PGRES_TUPLES_OK {
				let rows_affected_str = CStr::from_ptr(PQcmdTuples(res_ptr))
					.to_string_lossy()
					.into_owned();
				let rows_affected: i32 = rows_affected_str.parse().unwrap_or(0);
				return Ok(rows_affected)
			} else {
				let err_msg = CStr::from_ptr(PQresultErrorMessage(res_ptr)).to_string_lossy().into_owned();
				debug!("Lpsql.exec failed: {}", err_msg);
				PQclear(res_ptr);
				return Err(LpsqlError::ExecuteFailed(err_msg))
			}
		}
	}
	pub async fn fetch_one(&self, query: &str, params: Vec<SqlParam>)
		-> Result<Option<String>, LpsqlError>
	{
		match self.fetchall(query, params).await {
			Err(e) => {
				// Mirror fetchall's behavior: emit the failing query to stderr and
				// propagate the original error so callers can handle it appropriately.
				eprintln!("Error executing SQL: {}", query);
				debug!("fetch_one LpsqlError: {:?}", e);
				return Err(e)
			},
			Ok(v) => {
				if v.len() == 0 {
					return Ok(None)
				} else {
					return Ok(Some(v[0].to_string()))
				}
			}
		}
	}
	pub async fn close(&self) {
		unsafe {
			let mut conn_lock = self.conn.lock().await;
			let conn_ptr = (*conn_lock).as_mut() as *mut PGconn;
			PQfinish(conn_ptr);
		}
	}
}

fn hash_query(query: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.hash(&mut hasher);
    hasher.finish()
}
