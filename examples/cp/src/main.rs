#![feature(type_alias_impl_trait)]

use std::env;

use async_executor_util::PerThreadExecutor;
use reika_reactor::io;

async fn copy_file(src: &str, dest: &str) {
    let src = io::File::open(src).await.unwrap();
    let dest = io::File::options().create(true).write(true).open(dest).await.unwrap();

    let mut buf = [0; 4096];

    loop {
        let read = src.read(&mut buf).await.unwrap();
        let _ = dest.write(&mut buf[0..(read as usize)]).await.unwrap();

        if read < buf.len() as _ {
            break;
        }
    }

    src.close().await.unwrap();
    dest.close().await.unwrap();
}

#[reika_macros::task]
async fn entry() {
    let src = env::args()
        .nth(1)
        .expect("invalid number of args - USAGE: cp <src> <dest>");
    let dest = env::args()
        .nth(2)
        .expect("invalid number of args - USAGE: cp <src> <dest>");

    copy_file(src.as_str(), dest.as_str()).await;
}

fn main() {
    PerThreadExecutor::spawn_task(entry().unwrap());

    PerThreadExecutor::run(Some(|| {
        if reika_reactor::PerThreadReactor::flush(0, 0, false).is_err() {
            println!("oops, reactor failed");
        }
    }));
}

