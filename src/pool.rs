use crate::Lpsql;
use async_std::channel::{bounded, Sender, Receiver};
use std::time::Duration;
use async_std::task;
use crate::conf::Conf;

static DEF_CONN_TIMEOUT_SEC: u64 = 5;

pub struct ConnectionPool {
    sender: Sender<Lpsql>,
    receiver: Receiver<Lpsql>,
	conf: Conf,
	conn_timeout: Duration,
}

impl ConnectionPool {
    pub fn new(size: usize, conf_fname: Option<&str>, conn_timeout: Option<u64>) -> Self {
		let conf = get_conf(conf_fname);
		let conn_timeout = Duration::from_secs(conn_timeout.unwrap_or(DEF_CONN_TIMEOUT_SEC));
        let (sender, receiver) = bounded(size);
        for _ in 0..size {
			let conn = Lpsql::new(conf.clone(), conn_timeout.clone());
            sender.try_send(conn).unwrap();
        }
        let pool = ConnectionPool { sender, receiver, conf, conn_timeout };
		pool.start_cleanup();
		pool
    }

    pub async fn get_conn(&self) -> Lpsql {
        let mut conn = self.receiver.recv().await.unwrap();
		conn.touch();
		conn
    }

    pub async fn release_conn(&self, mut conn: Lpsql) {
		conn.touch();
        self.sender.send(conn).await.unwrap();
    }

	pub fn start_cleanup(&self) {
		let sender = self.sender.clone();
		let receiver = self.receiver.clone();
		let conf = self.conf.clone();
		let conn_timeout = self.conn_timeout.clone();

		task::spawn(async move {
			loop {
				task::sleep(Duration::from_secs(5)).await;
				let mut active_conns = vec![];
				while let Ok(conn) = receiver.try_recv() {
					if conn.is_timeout_exceed() {
						conn.close().await;
						active_conns.push(Lpsql::new(conf.clone(), conn_timeout.clone()));
					} else if conn.is_active().await == false {
						conn.close().await;
						active_conns.push(Lpsql::new(conf.clone(), conn_timeout.clone()));
					} else {
						active_conns.push(conn);
					}
				}
				for conn in active_conns {
					sender.send(conn).await.unwrap();
				}
			}
		});
	}
}


pub fn get_conf(conf_fname: Option<&str>) -> Conf {
	let conf_fname = conf_fname.unwrap_or("lpsql.toml");
	Conf::new(conf_fname)
}
