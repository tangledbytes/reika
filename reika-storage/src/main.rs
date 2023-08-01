use std::{thread, time::Duration};

use async_executor::Executor;

fn main() {
    let ex = Executor::new();
    ex.spawn(async {
        println!("Hello from async!");

        println!("Hello again!!");
    });

    ex.spawn(async {
        println!("Although last, I shall prevail thou know the secrets of the world");
    });

    ex.run(Some(|| thread::sleep(Duration::from_millis(100))));
}
