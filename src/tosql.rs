use std::ffi::{CString};
use std::ptr;
use serde_json::Value;
use chrono::{DateTime, Utc, NaiveDate};


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

    /// Текстовое представление параметра (None для NULL). Для логов и тестов.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            SqlParam::Text(ref s) => s.to_str().ok(),
            SqlParam::Null => None,
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

impl ToSql for f32 {
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

impl ToSql for &String {
    fn to_sql(&self) -> SqlParam {
		self.as_str().to_sql()
    }
}

impl ToSql for DateTime<Utc> {
    fn to_sql(&self) -> SqlParam {
        let s = self.to_rfc3339();
        if s.contains('\0') {
            SqlParam::Null
        } else {
            match CString::new(s) {
                Ok(c) => SqlParam::Text(c),
                Err(_) => SqlParam::Null,
            }
        }
    }
}

impl ToSql for &DateTime<Utc> {
    fn to_sql(&self) -> SqlParam {
        (*self).to_sql()
    }
}

impl ToSql for NaiveDate {
    fn to_sql(&self) -> SqlParam {
        // Serialize NaiveDate as "YYYY-MM-DD". Treat any unexpected NUL-containing
        // strings as NULL to avoid CString::new errors.
        let s = self.to_string();
        if s.contains('\0') {
            SqlParam::Null
        } else {
            match CString::new(s) {
                Ok(c) => SqlParam::Text(c),
                Err(_) => SqlParam::Null,
            }
        }
    }
}

impl ToSql for &NaiveDate {
    fn to_sql(&self) -> SqlParam {
        (*self).to_sql()
    }
}

impl ToSql for &[i64] {
    fn to_sql(&self) -> SqlParam {
        if self.is_empty() {
            return SqlParam::Text(CString::new("{}").unwrap());
        }
        let s = format!("{{{}}}", self.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        SqlParam::Text(CString::new(s).unwrap())
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
