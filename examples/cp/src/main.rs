#![feature(type_alias_impl_trait)]

use std::env;

use async_executor_util::PerThreadExecutor;
use reika_reactor::io;

async fn copy_file(src: &str, dest: &str) {
    let src = io::open(src, 0, 0).await.unwrap();
    let dest = io::open(
        dest,
        io::libc::O_CREAT | io::libc::O_WRONLY,
        0o777
    ).await.unwrap();

    let mut buf = [0; 4096];

    loop {
        let read = io::read(src, &mut buf).await.unwrap();
        let _ = io::write(dest, &mut buf[0..(read as usize)]).await.unwrap();

        if read < buf.len() as _ {
            break;
        }
    }

    let _res = io::close(src).await.unwrap();
    let _res = io::close(dest).await.unwrap();
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
        if reika_reactor::iouring::PerThreadReactor::flush(0, 0, false).is_err() {
            println!("oops, reactor failed");
        }
    }));
}

