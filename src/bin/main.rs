use lpsql;
use lpsql::QueryParam as qp;
//use tinyjson::JsonValue;
use serde_json;
use serde::{Serialize,Deserialize};


#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    id: Option<f64>,
    name: String,
    is_active: bool,
}
impl User {
}

impl User {
    pub fn all() -> Vec<User> {
        let mut structs: Vec<User> = vec![];
        let prms: Vec<qp> = vec![];
        let query = "select row_to_json(data) from (select * from users) data";
        let r = lpsql::exec(query, prms);
        for row in r.unwrap() {
            //println!("row: {row:?}\n");
            let u: User = serde_json::from_str(&row).unwrap();
            structs.push(u);
        }
        structs
    }
    fn _create(name: String, is_active: bool) {
        let prms: Vec<qp> = vec![
            qp::String(name),
            qp::Bool(is_active),
        ];
        let query = "insert into users (name, is_active) \
            values ($1::TEXT, $2::BOOL)";
        match lpsql::exec(query, prms) {
            Err(e) => println!("{e}"),
            Ok(v) => println!("{v:?}"),
        }
    }
}

fn main() {
    let prms: Vec<qp> = vec![
        qp::Number(7),
        qp::String("mia".into()),
        qp::Bool(true),
    ];
    let query = "select * from users where id = $1::INT \
                 and name = $2::TEXT and is_active = $3::BOOL";
    let r = lpsql::exec(query, prms);
    println!("SQL result: {r:?}");
    
    let _users = User::all();
    //let _users_s = serde_json::to_string(&users);
    //User::_create("serj".to_string(), true);
}
