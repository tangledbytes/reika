#![no_std]

mod queue;
mod util;
mod waker;

use core::future::Future;
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::{cell::UnsafeCell, ptr::NonNull};
use queue::{TaskFreeList, TaskQueue};
use util::UninitCell;

/// TaskHeader contains the raw data regarding any task, the tasks are an abstraction on top of
/// futures and hence the task header contains the raw data that is required to run a future.
pub(crate) struct TaskHeader {
    /// executor_queue_item is used to embed the task into executor's queue
    /// if there is any.
    executor_queue_item: queue::TaskQueueEmbedItem,

    /// executor is the reference to the executor that is running this task
    executor: UnsafeCell<Option<&'static Executor>>,

    /// poll_fn is the function which will be called whenever the executor of the task
    /// wishes to poll the underlying future.
    poll_fn: Option<unsafe fn(TaskRef) -> bool>,

    /// task_pool_queue_item is used to embed the task into task pool's free
    /// list.
    task_pool_queue_item: queue::TaskFreeListEmbedItem,

    /// task_pool_ptr is a const pointer to task pool.
    ///
    /// This should be `null` if a [TaskPool] was not used to create
    /// this Task (eg. Direct [TaskStorage] usage).
    task_pool_ptr: *const (),

    /// task_storage_ptr is a mut pointer to the storage which
    /// is holding this TaskHeader.
    ///
    /// This pointer can NEVER be null (other than before getting initialized)
    task_storage_ptr: *mut (),

    /// task_pool_finalizer_fn is triggered by the executor
    /// which will pass a const pointer to task pool and
    /// TaskRef.
    ///
    /// This should be None if a [TaskPool] was not used to create
    /// this Task (eg. Direct [TaskStorage] usage)
    task_pool_finalizer_fn: Option<unsafe fn(*const (), TaskRef)>,
}

/// TaskRef just holds a pointer to TaskHeader
///
/// This is intended to be used to pass around the TaskHeader pointer
/// without hassle of passing around raw pointers.
#[derive(Clone, Copy)]
pub struct TaskRef {
    ptr: NonNull<TaskHeader>,
}

impl TaskRef {
    /// new allows to create a new TaskRef from TaskStorage
    fn new<F: Future + 'static>(task: &'static TaskStorage<F>) -> Self {
        Self {
            ptr: NonNull::from(task).cast(),
        }
    }

    /// header returns a static reference to the internal TaskHeader
    pub(crate) fn header(&self) -> &'static TaskHeader {
        unsafe { self.ptr.as_ref() }
    }

    /// mut_header returns a mutable static reference to the internal TaskHeader
    pub(crate) fn mut_header(&mut self) -> &'static mut TaskHeader {
        unsafe { self.ptr.as_mut() }
    }

    /// as_ptr returns a const pointer to TaskHeader
    ///
    /// NOTE: TaskRef is supposed to be created ONLY via
    /// a *mut TaskHeader and casting *mut _ to *const _
    /// is sound.
    pub(crate) fn as_ptr(&self) -> *const TaskHeader {
        self.ptr.as_ptr()
    }

    pub(crate) unsafe fn enqueue_self(mut self) {
        let header = self.ptr.as_mut();
        let ex = *header.executor.get();
        if let Some(ex) = ex {
            ex.enqueue(self);
        }
    }

    /// # Safety
    /// The `ptr` must have been obtained from `TaskRef::as_ptr`.
    /// This is because a TaskRef can only be created via a
    /// *mut TaskHeader.
    pub(crate) unsafe fn from_ptr(ptr: *const TaskHeader) -> Self {
        Self {
            ptr: NonNull::new_unchecked(ptr as *mut TaskHeader),
        }
    }
}

/// Wake a task by `TaskRef`.
///
/// You can obtain a `TaskRef` from a `Waker` using [`task_from_waker`].
pub fn wake_task(task: TaskRef) {
    unsafe {
        task.enqueue_self();
    }
}

#[repr(C)]
pub struct TaskStorage<F: Future + 'static> {
    raw: TaskHeader,
    future: UninitCell<F>,
}

impl<F: Future + 'static> TaskStorage<F> {
    const NEW: Self = Self::new();

    pub const fn new() -> Self {
        Self {
            raw: TaskHeader {
                executor_queue_item: queue::TaskQueueEmbedItem::new(),
                task_pool_queue_item: queue::TaskFreeListEmbedItem::new(),
                executor: UnsafeCell::new(None),
                poll_fn: None,
                task_pool_ptr: core::ptr::null(),
                task_pool_finalizer_fn: None,
                task_storage_ptr: core::ptr::null_mut(),
            },
            future: UninitCell::uninit(),
        }
    }

    pub fn prepare_task(&'static mut self, future: impl FnOnce() -> F) -> TaskRef {
        // # Safety
        // This is safe to do because this is essentially a `ptr::write`
        // which is sound so for as long as the destination is valid and
        // the current pointer is valid
        unsafe {
            self.future.write(future());
        }

        self.raw.task_storage_ptr = self as *mut _ as *mut ();
        self.raw.poll_fn = Some(TaskStorage::<F>::poll);

        TaskRef::new(self)
    }

    unsafe fn poll(p: TaskRef) -> bool {
        let this = &mut *(p.as_ptr() as *mut TaskStorage<F>);
        let mut res = false;

        let future = Pin::new_unchecked(this.future.as_mut());
        let waker = waker::from_task(p);
        let mut ctx = Context::from_waker(&waker);
        match future.poll(&mut ctx) {
            Poll::Ready(_) => {
                this.future.drop_in_place();
                res = true;
            }
            Poll::Pending => {}
        }

        // the compiler is emitting a virtual call for waker drop, but we know
        // it's a noop for our waker.
        mem::forget(waker);

        res
    }
}

/// Raw storage that can hold up to N tasks of the same type.
///
/// This is essentially a `[TaskStorage<F>; N]`.
pub struct TaskPool<F: Future + 'static, const N: usize> {
    pool: [TaskStorage<F>; N],
    free_list: TaskFreeList,
    exhaust_list_cnt: usize,
}

impl<F: Future + 'static, const N: usize> TaskPool<F, N> {
    /// Create a new TaskPool, with all tasks in non-spawned state.
    pub const fn new() -> Self {
        Self {
            pool: [TaskStorage::NEW; N],
            free_list: TaskFreeList::new(),
            exhaust_list_cnt: 0,
        }
    }

    /// prepare_task consumes a future, stores it in one of the available [TaskStorage] and
    /// returns a [TaskRef] which points to a [TaskHeader] which points to the give future.
    pub fn prepare_task(&'static mut self, future: impl FnOnce() -> F) -> Option<TaskRef> {
        let self_ptr = self as *const _ as *const ();

        let storage = if self.exhaust_list_cnt < N {
            let storage = &mut self.pool[self.exhaust_list_cnt];
            self.exhaust_list_cnt += 1;

            Some(storage)
        } else if let Some(task) = unsafe { self.free_list.dequeue() } {
            let storage = task.header().task_storage_ptr as *mut TaskStorage<F>;

            assert!(!storage.is_null());

            unsafe { Some(&mut *storage) }
        } else {
            None
        };

        if let Some(storage) = storage {
            unsafe {
                storage.future.write(future());

                storage.raw.task_storage_ptr = storage as *mut _ as *mut ();
                storage.raw.poll_fn = Some(TaskStorage::<F>::poll);
                storage.raw.task_pool_ptr = self_ptr;
                storage.raw.task_pool_finalizer_fn = Some(TaskPool::<F, N>::finalize);

                Some(TaskRef::from_ptr(&storage.raw))
            }
        } else {
            None
        }
    }

    /// finalize consumes a raw pointer to [TaskPool] and a [TaskRef] which should yield a
    /// [TaskStorage]. This [TaskStorage] is then marked free for use again.
    ///
    /// It is intended that the executor should invoke this function once a task [Future]
    /// is completed.
    unsafe fn finalize(task_pool: *const (), t: TaskRef) {
        let task_pool = task_pool as *const TaskPool<F, N>;

        task_pool
            .as_ref()
            .unwrap()
            .free_list
            .enqueue(TaskRef::from_ptr(t.as_ptr()));
    }
}

/// Reika Async Executor
pub struct Executor {
    task_queue: TaskQueue,
    spawned: UnsafeCell<u64>,
}

impl Executor {
    /// new creates a new instance of executor
    pub const fn new() -> Self {
        Self {
            task_queue: TaskQueue::new(),
            spawned: UnsafeCell::new(0),
        }
    }

    /// spawn_task consumes a [TaskRef] and enqueues it for running
    ///
    /// This function relies on a TaskRef to already exist which can be
    /// created via static TaskStorage. This ensures that no dynamic memory
    /// allocation happens but this also makes this interface harder to consume
    pub fn spawn_task(&'static self, t: TaskRef) {
        // Increment the total spawned task here and not in the
        // enqueue function as that is shared by wakeup mechanism
        // as well.
        let spawned = self.spawned.get();
        unsafe {
            *spawned += 1;
        }

        self.enqueue(t);
    }

    /// run starts a busy loop and keep polling the tasks forever
    pub fn run(&'static self, mut post_drain_fn: Option<impl FnMut()>) {
        loop {
            // Drain the user tasks
            self.task_queue.drain(|mut taskptr| {
                let task = taskptr.mut_header();

                if let Some(poll) = task.poll_fn {
                    // # Safety: Implied
                    let finished = unsafe { poll(TaskRef::from_ptr(taskptr.as_ptr())) };
                    if finished {
                        if let Some(task_pool_finalizer) = task.task_pool_finalizer_fn {
                            // # Safety: Implied
                            unsafe {
                                task_pool_finalizer(task.task_pool_ptr, TaskRef::from_ptr(taskptr.as_ptr()))
                            }
                        }

                        let queued = self.spawned.get();
                        assert!(!queued.is_null());

                        unsafe { *queued -= 1; }
                    }
                }
            });

            // Execute post drain function
            if let Some(ref mut post_drain_fn) = post_drain_fn {
                post_drain_fn();
            }

            // If nothing is queued break
            unsafe {
                if *self.spawned.get() == 0 {
                    break;
                }
            };
        }
    }

    pub(crate) fn enqueue(&'static self, t: TaskRef) {
        unsafe {
            t.header().executor.get().replace(Some(self));
            self.task_queue.enqueue(t);
        }
    }
}
