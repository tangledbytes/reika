#![feature(type_alias_impl_trait)]

use async_executor_util::PerThreadExecutor;
use reika_reactor::net;

#[reika_macros::task(pool_size = 1024)]
async fn handle_connection_thread1(mut connection: reika_reactor::net::TcpStream) {
    loop {
        let mut buf = [0; 4096];
        if let Ok(read) = connection.read(&mut buf).await {
            if read == 0 {
                break;
            }

            if let Err(_) = connection.send(&buf[0..(read)]).await {
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

#[reika_macros::task]
async fn entry_thread1() {
    let listener = net::TcpListner::bind("127.0.0.1:2310", reika_reactor::net::SOMAXCONN)
        .await
        .unwrap();
    println!("Listening on 127.0.0.1:2310");

    loop {
        let connection = listener.accept().await.unwrap();

        loop {
            if let Some(task) = handle_connection_thread1(connection) {
                PerThreadExecutor::spawn_task(task);
                break;
            } else {
                reika_reactor::core::yield_now().await;
            }
        }
    }
}

fn main() {
    PerThreadExecutor::spawn_task(entry_thread1().unwrap());

    PerThreadExecutor::run(Some(|| {
        if reika_reactor::PerThreadReactor::flush(0, 0, false).is_err() {
            println!("oops, flush failed");
        }
        if reika_reactor::PerThreadReactor::run_for_ns(1000).is_err() {
            println!("oops, reactor failed");
        }
    }));
}
