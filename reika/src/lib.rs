extern crate libc;

pub mod executor {
    use std::future::Future;
    pub use async_executor as core;


    unsafe fn _make_static<T>(i: &T) -> &'static T {
        std::mem::transmute(i)
    }

    /// PerThreadExecutor exposes the exact same methods
    /// that are exposed by [Executor] as functions.
    ///
    /// The advantage is that the struct automatically
    /// manages the creation and destruction of executor and
    /// ensures that there is only one executor instance per
    /// thread (that's the design of reika async executor).
    pub struct PerThreadExecutor;

    impl PerThreadExecutor {
        thread_local! {
            static EXECUTOR: core::Executor = const { core::Executor::new() };
        }

        /// spawn_task consumes a task and spawns it to an executor
        /// running on the current thread.
        ///
        /// NOTE: TaskRef can be created from [TaskStorage] which can be
        /// created statically, this allows to create spawn tasks with
        /// zero runtime memory allocation.
        pub fn spawn_task(task: core::TaskRef) {
            Self::EXECUTOR.with(|ex: &core::Executor| {
                // # Safety: This is safe because this static is never
                // going to outlive the running thread.
                let static_ex = unsafe { _make_static(ex) };
                static_ex.spawn_task(task);
            });
        }

        /// spawn takes any future and spawns it to an executor running
        /// on the current thread.
        ///
        /// NOTE: This method does runtime memory allocation and will NEVER
        /// release the memory aquired for the storage of the future. This
        /// method should be used with care!
        pub fn spawn(fut: impl Future<Output = ()> + 'static) {
            Self::EXECUTOR.with(|ex: &core::Executor| {
                // # Safety: This is safe because this static is never
                // going to outlive the running thread.
                let static_ex = unsafe { _make_static(ex) };

                let boxed = Box::new(core::TaskStorage::new());
                let leaked = Box::leak(boxed);
                let task = leaked.prepare_task(|| fut);

                static_ex.spawn_task(task);
            });
        }

        /// run is the function that actually starts the executor
        ///
        /// It can take a `post_drain_fn` which is executed by the executor
        /// after it has finished running a set of spawns.
        pub fn run(post_drain_fn: Option<impl FnMut()>) {
            Self::EXECUTOR.with(|ex: &core::Executor| {
                // # Safety: This is safe because this static is never
                // going to outlive the running thread.
                let static_ex = unsafe { _make_static(ex) };
                static_ex.run(post_drain_fn);
            });
        }
    }
}

pub mod util {
    pub fn set_cpu_affinity(id: usize) -> bool {
        let mut cpuset = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };

        unsafe { libc::CPU_SET(id, &mut cpuset) };

        let res = unsafe {
            libc::sched_setaffinity(
                0,
                std::mem::size_of::<libc::cpu_set_t>(),
                &cpuset,
            )
        };

        res == 0
    }
}

pub mod macros {
    pub use reika_macros::*;
}

pub mod reactor {
    pub use reika_reactor::*;
}