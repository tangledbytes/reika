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

pub(crate) const STATE_NONE: u8 = 0;
pub(crate) const STATE_SPAWNED: u8 = 1 << 0;
pub(crate) const STATE_IN_QUEUE: u8 = 1 << 1;

/// TaskHeader contains the raw data regarding any task, the tasks are an abstraction on top of
/// futures and hence the task header contains the raw data that is required to run a future.
pub(crate) struct TaskHeader {
    /// state represents the current state of the task, these values
    /// could be either [STATE_NONE] or [STATE_SPAWNED] or [STATE_IN_QUEUE]
    state: u8,

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
                state: STATE_NONE,
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
        self.raw.state = STATE_SPAWNED;

        TaskRef::new(self)
    }

    unsafe fn poll(p: TaskRef) {
        let this = &mut *(p.as_ptr() as *mut TaskStorage<F>);

        let future = Pin::new_unchecked(this.future.as_mut());
        let waker = waker::from_task(p);
        let mut ctx = Context::from_waker(&waker);
        match future.poll(&mut ctx) {
            Poll::Ready(_) => {
                this.future.drop_in_place();

                this.raw.state &= !STATE_IN_QUEUE;
                this.raw.state &= !STATE_SPAWNED;
            }
            Poll::Pending => {}
        }

        // the compiler is emitting a virtual call for waker drop, but we know
        // it's a noop for our waker.
        mem::forget(waker);
    }
}

struct AvailableTask<F: Future + 'static> {
    task: &'static mut TaskStorage<F>,
}

impl<F: Future + 'static> AvailableTask<F> {
    fn claim(task: &'static mut TaskStorage<F>) -> Option<Self> {
        if task.raw.state != STATE_NONE {
            None
        } else {
            Some(Self { task })
        }
    }

    fn initialize(self, future: impl FnOnce() -> F) -> TaskRef {
        self.task.raw.poll_fn = Some(TaskStorage::<F>::poll);
        unsafe {
            self.task.future.write(future());
        }

        self.task.raw.state = STATE_SPAWNED;

        TaskRef::new(self.task)
    }
}

/// Raw storage that can hold up to N tasks of the same type.
///
/// This is essentially a `[TaskStorage<F>; N]`.
pub struct TaskPool<F: Future + 'static, const N: usize> {
    pool: [TaskStorage<F>; N],
}

impl<F: Future + 'static, const N: usize> TaskPool<F, N> {
    /// Create a new TaskPool, with all tasks in non-spawned state.
    pub const fn new() -> Self {
        Self {
            pool: [TaskStorage::NEW; N],
        }
    }

    pub fn prepare_task(&'static mut self, future: impl FnOnce() -> F) -> Option<TaskRef> {
        let task = self.pool.iter_mut().find_map(AvailableTask::claim);
        task.map(|task| task.initialize(future))
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
        let curr_state = t.header().state;

        if curr_state & STATE_IN_QUEUE == 0 {
            self.enqueue(t);
        }
    }

    /// run starts a busy loop and keep polling the tasks forever
    pub fn run(&'static self, mut post_drain_fn: Option<impl FnMut()>) -> ! {
        loop {
            // Drain the user tasks
            self.task_queue.drain(|mut taskptr| {
                let task = taskptr.mut_header();
                task.state |= STATE_IN_QUEUE;
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
