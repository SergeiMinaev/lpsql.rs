use std::ffi::{CString};
use std::ptr;
use serde_json::Value;


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

impl ToSql for i16 {
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
        // Treat IEEE-754 NaN as an absent value and encode it as SQL NULL.
        if self.is_nan() {
            SqlParam::Null
        } else {
            SqlParam::Text(CString::new(self.to_string()).unwrap())
        }
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

impl ToSql for Value {
    fn to_sql(&self) -> SqlParam {
        match self {
            Value::Null => SqlParam::Null,
            v => {
                let s = v.to_string();
                if s.contains('\0') {
                    // unlikely; treat as NULL to avoid CString::new error
                    SqlParam::Null
                } else {
                    SqlParam::Text(CString::new(s).unwrap())
                }
            }
        }
    }
}
