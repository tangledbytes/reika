// #![feature(local_key_cell_methods)]
#![feature(type_alias_impl_trait)]

use async_executor_util::PerThreadExecutor;

#[reika_macros::task]
async fn entry() {
    read_file("reika-storage/src/main.rs").await;
}

async fn read_file(path: &str) {
    println!("reading file");
    let res = reika_reactor::ops::fs::open(path, 0).await.unwrap();
    let mut buf = [0; 4096];
    println!("Okay, opened the file: {res}");

    loop {
        let read = reika_reactor::ops::fs::read(res, &mut buf).await.unwrap();

        println!(
            "{}",
            std::str::from_utf8(&buf).expect("expected to get valid utf8")
        );

        if read < buf.len() as _ {
            println!("finished reading");
            break;
        }
    }

    let _res = reika_reactor::ops::fs::close(res).await.unwrap();
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
    PerThreadExecutor::spawn_task(entry().unwrap());

    PerThreadExecutor::run(Some(|| {
        let rx = unsafe { reika_reactor::iouring::Reactor::get_static() };
        if rx.run_for_ns(10000).is_err() {
            println!("oops, reactor failed");
        }
    }));
}
