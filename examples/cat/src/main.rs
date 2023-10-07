#![feature(type_alias_impl_trait)]

use std::env;

use reika::executor::PerThreadExecutor;
use reika::reactor::io;

async fn read_file(path: &str) {
    let file = io::File::open(path).await.unwrap();

    let mut buf = [0; 4096];

    loop {
        let read = file.read(&mut buf).await.unwrap();

        print!(
            "{}",
            std::str::from_utf8(&buf[0..(read as usize)]).expect("expected to get valid utf8")
        );

        if read < buf.len() as _ {
            break;
        }
    }

    file.close().await.unwrap();
}

#[reika::macros::task]
async fn entry() {
    let path = env::args()
        .nth(1)
        .expect("invalid number of args - USAGE: cat <path>");

    read_file(path.as_str()).await;
}

fn main() {
    PerThreadExecutor::spawn_task(entry().unwrap());

    PerThreadExecutor::run(Some(|| {
        if reika::reactor::PerThreadReactor::run_for_ns(0).is_err() {
            println!("oops, reactor failed");
        }
    }));
}
