#![no_std]

mod queue;
mod spawner;
mod util;
mod waker;

use core::future::Future;
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::{cell::UnsafeCell, ptr::NonNull};
use queue::TaskQueue;
use util::UninitCell;

/// TaskHeader contains the raw data regarding any task, the tasks are an abstraction on top of
/// futures and hence the task header contains the raw data that is required to run a future.
pub(crate) struct TaskHeader {
    /// queue_item is used to embed the task into executor's queue
    /// if there is any.
    queue_item: queue::TaskQueueEmbedItem,

    /// executor is the reference to the executor that is running this task
    executor: UnsafeCell<Option<&'static Executor>>,

    /// poll_fn is the function which will be called whenever the executor of the task
    /// wishes to poll the underlying future.
    poll_fn: Option<unsafe fn(TaskRef)>,
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
    pub const fn new() -> Self {
        Self {
            raw: TaskHeader {
                queue_item: queue::TaskQueueEmbedItem::new(),
                executor: UnsafeCell::new(None),
                poll_fn: None,
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

        self.raw.poll_fn = Some(TaskStorage::<F>::poll);

        TaskRef::new(self)
    }

    unsafe fn poll(p: TaskRef) {
        let this = &*(p.as_ptr() as *const TaskStorage<F>);

        let future = Pin::new_unchecked(this.future.as_mut());
        let waker = waker::from_task(p);
        let mut ctx = Context::from_waker(&waker);
        match future.poll(&mut ctx) {
            Poll::Ready(_) => {
                this.future.drop_in_place();
            }
            Poll::Pending => {}
        }

        // the compiler is emitting a virtual call for waker drop, but we know
        // it's a noop for our waker.
        mem::forget(waker);
    }
}

/// Reika Async Executor
pub struct Executor {
    task_queue: TaskQueue,
}

impl Executor {
    /// new creates a new instance of executor
    pub const fn new() -> Self {
        Self {
            task_queue: TaskQueue::new(),
        }
    }

    /// spawn_task consumes a `[TaskRef]` and enqueues it for running
    ///
    /// This function relies on a TaskRef to already exist which can be
    /// created via static TaskStorage. This ensures that no dynamic memory
    /// allocation happens but this also makes this interface harder to consume
    pub fn spawn_task(&'static self, t: TaskRef) {
        self.enqueue(t);
    }

    /// run starts a busy loop and keep polling the tasks forever
    pub fn run(&'static self, mut post_drain_fn: Option<impl FnMut()>) -> ! {
        loop {
            // Drain the user tasks
            self.task_queue.drain(|taskptr| {
                let task = taskptr.header();
                if let Some(poll) = task.poll_fn {
                    // # Safety: Implied
                    unsafe { poll(TaskRef::from_ptr(taskptr.as_ptr())) }
                }
            });

            // Execute post drain function
            if let Some(ref mut post_drain_fn) = post_drain_fn {
                post_drain_fn();
            }
        }
    }

    pub(crate) fn enqueue(&'static self, t: TaskRef) {
        unsafe {
            t.header().executor.get().replace(Some(self));
            self.task_queue.enqueue(t);
        }
    }
}
