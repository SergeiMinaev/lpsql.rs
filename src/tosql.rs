use std::ffi::{CString};
use std::ptr;


#[derive(Clone)]
pub enum SqlParam {
    Text(CString),
    Null,
}

impl SqlParam {
    #[inline]
    pub fn as_ptr(&self) -> *const i8 {
        match self {
            SqlParam::Text(ref s) => s.as_ptr(),
            SqlParam::Null => ptr::null(),
        }
    }
}

pub trait ToSql {
    fn to_sql(&self) -> SqlParam;
}

impl ToSql for i32 {
    fn to_sql(&self) -> SqlParam {
        SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl ToSql for u32 {
    fn to_sql(&self) -> SqlParam {
        SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl ToSql for i64 {
    fn to_sql(&self) -> SqlParam {
        SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl ToSql for f64 {
    fn to_sql(&self) -> SqlParam {
        SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl ToSql for String {
    fn to_sql(&self) -> SqlParam {
		SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl ToSql for &str {
    fn to_sql(&self) -> SqlParam {
		SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl ToSql for bool {
    fn to_sql(&self) -> SqlParam {
        SqlParam::Text(CString::new(self.to_string()).unwrap())
    }
}

impl<T: ToSql> ToSql for Option<T> {
    fn to_sql(&self) -> SqlParam {
        match self {
            Some(v) => v.to_sql(),
            None => SqlParam::Null,
        }
    }
}
