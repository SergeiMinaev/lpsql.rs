use std::ffi::{CString, CStr};
use std::os::raw::c_char;
use std::{str, ptr};
use rand::{thread_rng, Rng};
use pq_sys::*;
use crate::conf::CONF;
use pq_sys::ConnStatusType::CONNECTION_OK;
use pq_sys::ExecStatusType::PGRES_COMMAND_OK;
use pq_sys::ExecStatusType::PGRES_TUPLES_OK;
pub mod conf;


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


pub fn _exec(query: &str, params: Vec<QueryParam>
                     ) -> Result<Vec<String>, String> {
    let conf = CONF.read().unwrap();
    let conninfo = CString::new(format!(
        "dbname={} user={} password={}", conf.dbname, conf.user, conf.password
    )).unwrap();
    unsafe {
        let conn = PQconnectdb(conninfo.as_ptr());
        if PQstatus(conn) != CONNECTION_OK {
            return result_and_finish(conn, Err("Unable to establish connection".to_string()))
        }
        let stmt_name = CString::new(
            thread_rng().gen_range(0..9999).to_string()).unwrap();
        let stmt_name_c: *const c_char = stmt_name.as_ptr() as *const c_char;
        let stmt = CString::new(query).unwrap();
        let stmt_c: *const c_char = stmt.as_ptr() as *const c_char;
        let n_params: ::std::os::raw::c_int = params.len().try_into().unwrap();
        let param_types: *const Oid = ptr::null();
        let prepare_res = PQprepare(conn, stmt_name_c, stmt_c, n_params, param_types);
        if PQresultStatus(prepare_res) == PGRES_COMMAND_OK {
            let n_params: ::std::os::raw::c_int = params.len().try_into().unwrap();
            let mut p_vec: Vec<String> = vec![];
            for p in params { p_vec.push(p.to_string()); }
            let p_vecc: Vec<CString> = p_vec
                .iter()
                .map(|arg| CString::new(arg.as_str()).unwrap())
                .collect();
            let p_vecc_ptr: Vec<_> = p_vecc.iter().map(|arg| arg.as_ptr()).collect();
            let param_vals: *const *const c_char = p_vecc_ptr.as_ptr();
            let param_lengths: *const i32 = ptr::null();
            let param_formats: *const i32 = ptr::null();
            let result_format = 0;
            let res = PQexecPrepared(conn, stmt_name_c, n_params, param_vals,
                param_lengths, param_formats, result_format);
            match PQresultStatus(res) {
                PGRES_TUPLES_OK => {
                    let num_rows = PQntuples(res);
                    let num_cols = PQnfields(res);
                    let mut r: Vec<String> = vec![];
                    for row_idx in 0..num_rows {
                        for col_idx in 0..num_cols {
                            //let field_name = CStr::from_ptr(PQfname(res, col_idx))
                            //    .to_str().expect("cant make field_name");
                            //let col_type_id = PQftype(res, col_idx);
                            let value_ptr = PQgetvalue(res, row_idx, col_idx);
                            let field_val = CStr::from_ptr(value_ptr)
                                .to_str().expect("Can't make field_val");
                            r.push(field_val.to_string());
                        }
                    }
                    return result_and_finish(conn, Ok(r));
                },
                PGRES_COMMAND_OK => {
                    return result_and_finish(conn, Ok(vec![]));
                },
                _ => {
                    let err_msg = CStr::from_ptr(PQresultErrorMessage(res));
                    return result_and_finish(
                        conn, Err(format!("PQExecPrepared err: {err_msg:?}")));
                }
            }
        } else {
            let st = PQresultStatus(prepare_res);
            println!("PQprepare status: '{st:?}' for query: '{query}'");
            let err_msg = CStr::from_ptr(PQresultErrorMessage(prepare_res));
            return result_and_finish(conn, Err(format!("PQprepare err: {err_msg:?}")));
        }
    }
}
pub fn result_and_finish(conn: *mut PGconn, res: Result<Vec<String>, String>
                         ) -> Result<Vec<String>, String> {
    unsafe {
        let _ = PQfinish(conn);
    }
    return res
}
pub fn get_one(query: &str, params: Vec<QueryParam>
                     ) -> Option<String> {
    match _exec(query, params) {
        Err(_) => None,
        Ok(v) => {
            if v.len() == 0 {
                return None
            } else {
                return Some(v[0].to_string())
            }
        }
    }
}

pub fn exec(query: &str, params: Vec<QueryParam>) -> bool {
    match _exec(query, params) {
        Err(e) => {
            println!("LPSQL ERR: {e}");
            return false
        },
        Ok(_) => true
    }
}
