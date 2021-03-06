use log::{debug,info,error};
use simplelog::*;
use pq_sys::*;
use std::ffi::{CString, CStr};
use std::{slice, str, ptr};
use chrono::prelude::*;
use tinyjson::JsonValue;
use core::fmt::Error;

// pg types
//    16 = Bool                 
//    17 = Bytea                
//    18 = Char                 
//    19 = Name                 
//    20 = Int8                 
//    21 = Int2                 
//    23 = Int4                 
//    24 = Regproc              
//    25 = Text                 
//    26 = Oid                  
//    27 = Tid                  
//    28 = Xid                  
//    29 = Cid                  
//   142 = Xml                  
//   600 = Point                
//   601 = Lseg                 
//   602 = Path                 
//   603 = Box                  
//   604 = Polygon              
//   628 = Line                 
//   650 = Cidr                 
//   700 = Float4               
//   701 = Float8               
//   702 = Abstime              
//   703 = Reltime              
//   704 = Tinterval            
//   705 = Unknown              
//   718 = Circle               
//   790 = Money                
//   829 = Macaddr              
//   869 = Inet                 
//  1042 = Bpchar               
//  1043 = Varchar              
//  1082 = Date                 
//  1083 = Time                 
//  1114 = Timestamp            
//  1184 = TimestampWithTimeZone
//  1186 = Interval             
//  1266 = TimeWithTimeZone     
//  1560 = Bit                  
//  1562 = Varbit               
//  1700 = Numeric              
//  1790 = Refcursor            
//  2249 = Record               
//  2278 = Void
// I've added this
//  3802 = JSON



pub fn execute(query: &str) -> Result<Vec<String>, &'static str> {
    CombinedLogger::init(
        vec![TermLogger::new(LevelFilter::Info, Config::default(),
        TerminalMode::Mixed, ColorChoice::Auto)]
    ).unwrap();
    let is_debug = false;
    let conninfo = CString::new("dbname=target_zen user=sm password=123")
        .expect("CString::new failed");
    unsafe {
        let conn = PQconnectdb(conninfo.as_ptr());
        debug!(">>> conn: {:?}", PQstatus(conn));
        if PQstatus(conn) != CONNECTION_OK {
            panic!("Unable to establish connection");
        }
        let query = CString::new(query).unwrap();
        let res = PQexec(conn, query.as_ptr());
        if PQresultStatus(res) != PGRES_TUPLES_OK {
            PQclear(res);
            return Err("Unknown error occured");
        } else {
            let num_rows = PQntuples(res);
            let num_cols = PQnfields(res);
            info!("Num rows {:?}, num cols: {:?}", num_rows, num_cols);

            let mut r: Vec<String> = vec![];
            for row_idx in 0..num_rows {
                info!("\nrow idx {}", row_idx);
                for col_idx in 0..num_cols {
                    let field_name = CStr::from_ptr(PQfname(res, col_idx))
                        .to_str().expect("cant make field_name");
                    let col_type_id = PQftype(res, col_idx);
                    let value_ptr = PQgetvalue(res, row_idx, col_idx);
                    let field_val = CStr::from_ptr(value_ptr)
                        .to_str().expect("cant make field_val");
                    r.push(field_val.to_string());
            //        if field_val != "" {
            //            if col_type_id == 16 {
            //                if field_val == "t" { field_val = "true"; }
            //                else { field_val = "false"; }
            //                let field_val: bool = field_val.parse().unwrap();
            //            } else if col_type_id == 23 {
            //                let field_val = field_val.parse::<i32>().unwrap();
            //            } else if col_type_id == 1043 {
            //            } else if col_type_id == 1184 {
            //                let field_val = DateTime::parse_from_str(
            //                    field_val, "%Y-%m-%d %H:%M:%S %#z"
            //                ).unwrap();
            //            } else if col_type_id ==  3802 {
            //                debug!(">>> RAW JSON STR{:?}", field_val);
            //                let json: JsonValue = field_val.parse().unwrap();
            //                debug!(">>> JSON {:?}", json);
            //                let json_str = json.stringify().unwrap();
            //                debug!(">>> JSON STR {:?}", json_str);
            //            } else {
            //                debug!("UNKNOWN TYPE ID: {}", col_type_id);
            //            }
            //        }
            //        debug!(">>> parsed value: {}", field_val);
                }
            }
            return Ok(r);
        }
    }
}
