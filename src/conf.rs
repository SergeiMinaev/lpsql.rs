use std::fs::File;
use std::io::Read;
use std::path::Path;
use once_cell::sync::Lazy;
use std::sync::RwLock;
use serde::Deserialize;


pub static CONF: Lazy<RwLock<Conf>> = Lazy::new(|| {
    RwLock::new(Conf::new("lpsql.toml"))
});

#[derive(Debug, Deserialize, Clone)]
pub struct Conf {
    pub dbname: String,
    pub user: String,
    pub password: String,
}

impl Conf {
    pub fn new(conf_fname: &str) -> Self {
        let path = Path::new(conf_fname);
        match File::open(&path) {
			Err(e) => {
				panic!("Unable to open 'lpsql.toml': {e:}");
			},
			Ok(mut file) => {
				let mut content = String::new();
				file.read_to_string(&mut content).unwrap();
				let conf: Conf = toml::from_str(&content).unwrap();
				return conf;
			}
		};
    }
}
