use std::ffi::{CString, CStr};


pub trait ToSql {
    fn to_sql(&self) -> CString;
}

impl ToSql for i32 {
    fn to_sql(&self) -> CString {
        CString::new(self.to_string()).unwrap()
    }
}

impl ToSql for u32 {
    fn to_sql(&self) -> CString {
        CString::new(self.to_string()).unwrap()
    }
}

impl ToSql for i64 {
    fn to_sql(&self) -> CString {
        CString::new(self.to_string()).unwrap()
    }
}

impl ToSql for f64 {
    fn to_sql(&self) -> CString {
        CString::new(self.to_string()).unwrap()
    }
}

impl ToSql for String {
    fn to_sql(&self) -> CString {
		CString::new(self.to_string()).unwrap()
    }
}

impl ToSql for &str {
    fn to_sql(&self) -> CString {
		CString::new(self.to_string()).unwrap()
    }
}

impl ToSql for bool {
    fn to_sql(&self) -> CString {
        CString::new(self.to_string()).unwrap()
    }
}
