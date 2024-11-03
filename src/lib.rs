use pq_sys::ConnStatusType::CONNECTION_OK;
use pq_sys::ExecStatusType::PGRES_COMMAND_OK;
use pq_sys::ExecStatusType::PGRES_TUPLES_OK;
use pq_sys::ExecStatusType::PGRES_BAD_RESPONSE;
use pq_sys::ExecStatusType::PGRES_FATAL_ERROR;
use std::hash::{Hash, Hasher};
use std::hash::DefaultHasher;
pub mod conf;
use std::ffi::{CString, CStr};
use std::{str, ptr};
use rand::{thread_rng, Rng};
use pq_sys::*;
use crate::conf::Conf;
use smol::io;
use std::sync::Arc;
use async_std::sync::Mutex;
use std::boxed::Box;
use std::time::{Instant};
use std::time::Duration;

pub mod pool;


pub enum QueryParam {
    Number(i32),
    String(String),
    Bool(bool),
}

impl QueryParam {
    pub fn to_string(&self) -> String {
        match self {
            QueryParam::Number(n) => n.to_string(),
            QueryParam::String(s) => s.to_string(),
            QueryParam::Bool(s) => s.to_string(),
        }
    }
}


pub struct Lpsql {
    pub conf: Conf,
	pub conn: Arc<Mutex<Box<PGconn>>>,
    pub last_used: Instant,
	pub conn_timeout: Duration,
}

impl Lpsql {
	pub fn new(conf: Conf, conn_timeout: Duration) -> Self {
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

	pub async fn exec(&self, query: &str, params: Vec<QueryParam>) -> io::Result<Vec<String>> {
		let conn_ptr = {
			let mut conn_lock = self.conn.lock().await;
			(*conn_lock).as_mut() as *mut PGconn
		};

		unsafe {
			if PQstatus(conn_ptr) != CONNECTION_OK {
				return Err(io::Error::new(io::ErrorKind::Other, "Connection failed"));
			}

			let stmt_name = CString::new(format!("stmt_{}", hash_query(query))).unwrap();
			let stmt = CString::new(query).unwrap();
			let n_params = params.len() as i32;

			let prepare_res = PQprepare(
				conn_ptr, stmt_name.as_ptr(), stmt.as_ptr(), n_params, ptr::null()
			);

			match PQresultStatus(prepare_res) {
				PGRES_COMMAND_OK => {
					PQclear(prepare_res);
				},
				PGRES_BAD_RESPONSE => {
					let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
						.to_string_lossy().into_owned();
					PQclear(prepare_res);
					return Err(io::Error::new(io::ErrorKind::Other, format!("Bad response during prepare: {}", err_msg)));
				},
				PGRES_FATAL_ERROR => {
					// Prepared query is probably already exists.
					let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
						.to_string_lossy().into_owned();
					PQclear(prepare_res);
					if err_msg.contains("already exists") {
						// Ignore if prepared query already exists.
						//println!("Prepared statement already exists: {}", err_msg);
					} else {
						return Err(io::Error::new(io::ErrorKind::Other, format!("Failed to prepare statement: {}", err_msg)));
					}
				},
				_ => {
					let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
						.to_string_lossy().into_owned();
					PQclear(prepare_res);
					return Err(io::Error::new(io::ErrorKind::Other, format!("Unexpected error during prepare: {}", err_msg)));
				}
			}

			// Execute prepared query
			let param_vals: Vec<_> = params.into_iter().map(|p| CString::new(p.to_string()).unwrap()).collect();
			let p_vecc_ptr: Vec<_> = param_vals.iter().map(|arg| arg.as_ptr()).collect();
			let res = PQexecPrepared(conn_ptr, stmt_name.as_ptr(), n_params, p_vecc_ptr.as_ptr(), ptr::null(), ptr::null(), 0);

			// Handle result
			if PQresultStatus(res) == PGRES_TUPLES_OK {
				let mut results = Vec::new();
				let num_rows = PQntuples(res);
				let num_cols = PQnfields(res);

				for row_idx in 0..num_rows {
					for col_idx in 0..num_cols {
						let value_ptr = PQgetvalue(res, row_idx, col_idx);
						let field_val = CStr::from_ptr(value_ptr).to_string_lossy().into_owned();
						results.push(field_val);
					}
				}

				PQclear(res);
				return Ok(results);
			} else {
				let err_msg = CStr::from_ptr(PQresultErrorMessage(res))
					.to_string_lossy().into_owned();
				PQclear(res);
				return Err(io::Error::new(io::ErrorKind::Other, format!("Query error: {}", err_msg)));
			}
		}
	}

	pub async fn _exec(&self, query: &str, params: Vec<QueryParam>) -> io::Result<Vec<String>> {
		unsafe {
			let conn_ptr = {
				let mut conn_lock = self.conn.lock().await;
				(*conn_lock).as_mut() as *mut PGconn
			};

			if PQstatus(conn_ptr) != CONNECTION_OK {
				return Err(io::Error::new(io::ErrorKind::Other, "Connection failed"));
			}

			let stmt_name = CString::new(thread_rng().gen_range(0..9999).to_string()).unwrap();
			let stmt = CString::new(query).unwrap();
			let n_params = params.len() as i32;
			let prepare_res = PQprepare(
				conn_ptr, stmt_name.as_ptr(), stmt.as_ptr(), n_params, ptr::null()
			);

			if PQresultStatus(prepare_res) != PGRES_COMMAND_OK {
				let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res))
					.to_string_lossy().into_owned();
				PQclear(prepare_res);
				return Err(io::Error::new(
					io::ErrorKind::Other, format!("Failed to prepare statement: {}", err_msg))
				);
			}

			let param_vals: Vec<_> = params.into_iter().map(|p| CString::new(p.to_string())
				.unwrap()).collect();
			let p_vecc_ptr: Vec<_> = param_vals.iter().map(|arg| arg.as_ptr()).collect();
			let res = PQexecPrepared(
				conn_ptr, stmt_name.as_ptr(), n_params, p_vecc_ptr.as_ptr(),
				ptr::null(), ptr::null(), 0
			);

			if PQresultStatus(res) == PGRES_TUPLES_OK {
				let mut results = Vec::new();
				let num_rows = PQntuples(res);
				let num_cols = PQnfields(res);

				for row_idx in 0..num_rows {
					for col_idx in 0..num_cols {
						let value_ptr = PQgetvalue(res, row_idx, col_idx);
						let field_val = CStr::from_ptr(value_ptr).to_string_lossy().into_owned();
						results.push(field_val);
					}
				}

				PQclear(res);
				return Ok(results);
			} else {
				let err_msg = CStr::from_ptr(PQresultErrorMessage(res))
					.to_string_lossy().into_owned();
				PQclear(res);
				return Err(io::Error::new(io::ErrorKind::Other, format!("Query error: {}", err_msg)));
			}
		}
	}

	pub async fn get_one(&self, query: &str, params: Vec<QueryParam>) -> Option<String> {
		match self.exec(query, params).await {
			Err(e) => {
				println!("get_one err: {e}");
				return None
			},
			Ok(v) => {
				if v.len() == 0 {
					return None
				} else {
					return Some(v[0].to_string())
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
