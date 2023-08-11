#![feature(type_alias_impl_trait)]

use std::{env, process, str::FromStr};

use async_executor_util::PerThreadExecutor;
use reika_reactor::io;

async fn read_file(path: &str) {
    let res = io::open(path, 0, 0).await.unwrap();
    let mut buf = [0; 4096];

    let mut total_read = 0i32;
    loop {
        let read = io::read(res, &mut buf, total_read as u64, 0).await.unwrap();
        total_read += read;

        print!(
            "{}",
            std::str::from_utf8(&buf[0..(read as usize)]).expect("expected to get valid utf8")
        );

        if read < buf.len() as _ {
            break;
        }
    }

    let _res = io::close(res).await.unwrap();
}

#[reika_macros::task]
async fn entry() {
    let path = env::args()
        .nth(1)
        .expect("invalid number of args - USAGE: cat <path>");

    read_file(path.as_str()).await;
}

fn main() {
    PerThreadExecutor::spawn_task(entry().unwrap());

    PerThreadExecutor::run(Some(|| {
        let rx = unsafe { reika_reactor::iouring::Reactor::get_static() };
        if rx.run_for_ns(1000).is_err() {
            println!("oops, reactor failed");
        }
    }));
}
