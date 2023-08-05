// #![feature(local_key_cell_methods)]
#![feature(type_alias_impl_trait)]

use std::future::Future;
use std::os::fd::RawFd;
use std::thread;
use std::time::Duration;

use async_executor_util::PerThreadExecutor;
use reika_reactor::error::InitFail;
use reika_reactor::iouring::Reactor;

unsafe fn _make_static<T>(i: &T) -> &'static T {
    std::mem::transmute(i)
}

thread_local! {
    static REACTOR: Result<Reactor, InitFail> = Reactor::new(1024);
}

unsafe fn get_static_reactor() -> &'static Reactor {
    REACTOR.with(|reactor_res: &Result<Reactor, InitFail>| {
        let reactor = reactor_res.as_ref().unwrap();

        _make_static(reactor)
    })
}

#[reika_macros::task]
async fn entry() {
    println!("entry.entry");
    let mut count = 0;
    loop {
        read_file("/home/utkarsh.linux/dev/reika/reika-storage/src/main.rs").await;
        count += 1;
        println!("{count}");
    }
    println!("entry.exit");
}

async fn read_file(path: &str) {
    unsafe {
        println!("reading file");
        let res = reika_reactor::ops::fs::Open::new(get_static_reactor(), path).await;
        if res <= 0 {
            println!("something has happened: {res}");
            return;
        }
        let mut buf = [0; 4096];
        println!("Okay, opened the file: {res}");

        loop {
            let read = reika_reactor::ops::fs::Read::new(get_static_reactor(), res, &mut buf).await;
            if read <= 0 {
                println!("breaking due to: {read}");
                break;
            }

            // println!(
            //     "{}",
            //     std::str::from_utf8(&buf).expect("expected to get valid utf8")
            // );

            if read < buf.len() as _ {
                println!("finished reading");
                break;
            }
        }

        let res = reika_reactor::ops::fs::Close::new(get_static_reactor(), res).await;
        if res < 0 {
            println!("could not close file: {res}");
        }
    };
}

// macro_rules! taskifier {
//     ($name:ident $b:block) => {
//         {
//             #[reika_macros::task]
//             async fn $name() {
//                 async $b.await;
//             }

//             $name()
//         }
//     };
// }

fn main() {
    // let a = taskifier!(coolfn {
    //     println!("From taskifier");
    // });

    // thread::spawn(|| {
    //     PerThreadExecutor::spawn(async {
    //         println!("Hello from async 2 spawn 1");

    //         PerThreadExecutor::spawn(async {
    //             println!("parent 2 spawned me for no reason");
    //         });
    //     });

    //     PerThreadExecutor::spawn(async {
    //         println!("Hello from async 2 spawn 2");
    //     });

    //     PerThreadExecutor::run(Some(|| {
    //         thread::sleep(Duration::from_millis(20 * 1000));
    //     }));
    // });

    // PerThreadExecutor::spawn(async {
    //     println!("Hello from async 1 spawn 1");

    //     PerThreadExecutor::spawn(async {
    //         println!("parent spawned me for no reason");
    //     });
    // });

    // PerThreadExecutor::spawn(async {
    //     println!("Hello from async 1 spawn 2");
    // });

    PerThreadExecutor::spawn_task(entry().unwrap());

    PerThreadExecutor::run(Some(|| {
        let rx = unsafe { get_static_reactor() };
        if rx.run_for_ns(10000).is_err() {
            println!("oops, reactor failed");
        }
    }));
}
