//use serde::{Serialize,Deserialize};
//use lpsql::QueryParam as qp;
//use lpsql::Lpsql;
//use lpsql::pool::ConnectionPool;
use futures_lite::future;
//use std::time::Duration;
//use lpsql::db::get_pool;
//use async_std::task;
//use std::sync::Arc;


//async fn fetch_users(pool: Arc<ConnectionPool>) {
	//let conn = pool.get_conn().await;
	//let prms: Vec<qp> = vec![
	//	//qp::Number(1),
	//];
	////let q = "select id from users_users where id > $1::INT;";
	//let q = "select pg_sleep(5)";
	//let r = conn.exec(q, prms).await;
	//pool.release_conn(conn).await;
	//println!("done: {r:?}");
//}

async fn amain() {
	//let pool = get_pool().clone();
	//let mut tasks = vec![];

    //for _ in 0..22 {
    //    let pool_clone = pool.clone();
    //    let task = task::spawn(async move {
    //        fetch_users(pool_clone).await;
    //    });
    //    tasks.push(task);
    //}

    //for task in tasks {
    //    task.await;
    //}
}

fn main() {
	future::block_on(amain());
}
