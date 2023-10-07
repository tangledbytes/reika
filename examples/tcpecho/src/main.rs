#![feature(type_alias_impl_trait)]

use reika::executor::PerThreadExecutor;
use reika::reactor::{core, net};

#[reika::macros::entry(replicate = 2)]
async fn main() {
    #[reika::macros::task(pool_size = 5000)]
    async fn connection_pool(mut connection: net::TcpStream) {
        loop {
            let mut buf = [0; 1024];
            if let Ok(read) = connection.read(&mut buf).await {
                if read == 0 {
                    break;
                }

                if connection.send(&buf[0..(read)]).await.is_err() {
                    break;
                }
            } else {
                break;
            }
        }

        match connection.close().await {
            Ok(_) => {}
            Err(_) => {
                println!("oops")
            }
        }
    }

    let listener = net::TcpListner::bind("127.0.0.1:2310", net::SOMAXCONN)
        .await
        .unwrap();
    println!("Listening on 127.0.0.1:2310");

    loop {
        let connection = listener.accept().await.unwrap();
        loop {
            match connection_pool(connection) {
                Some(task) => {
                    PerThreadExecutor::spawn_task(task);
                    break;
                }
                None => core::yield_now().await,
            }
        }
    }
}
