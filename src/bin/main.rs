use lpsql;
use tinyjson::JsonValue;

fn main() {
    let r = lpsql::execute("SELECT json_build_object(\
        'start_dt', start_dt, 'comment', comment, 'id', id, 'is_busy', is_busy) \
        from ads_adprojectblock limit 3");
    for row in r.unwrap() {
        let mut j: JsonValue = row.parse().unwrap();
        println!(">>> {:?}", j["is_busy"]);
    }
}
