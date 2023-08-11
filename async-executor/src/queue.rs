use crate::{TaskHeader, TaskRef};

use core::{
    cell::UnsafeCell,
    ptr::{null_mut, replace, NonNull},
};

/// TaskQueueEmbedItem should be embedded into any struct that needs to be
/// enqueued into the task queue.
pub(crate) struct TaskQueueEmbedItem {
    next: UnsafeCell<Option<TaskRef>>,
}
impl TaskQueueEmbedItem {
    pub const fn new() -> Self {
        Self {
            next: UnsafeCell::new(None),
        }
    }
}

/// TaskQueue is not a traditional queue. It does NOT do FIFO rather is
/// more like a stack where the dequeue operation is done on the last
/// element that was enqueued.
///
/// This is not really a problem because the goal for this queue is to support
/// scheduling of tasks in the executor and executor is _free_ to choose tasks
/// in any order it wishes to.
///
/// The queue is intentionally not thread safe because the executor itself is
/// single threaded.
pub struct TaskQueue {
    /// head is a pointer to the _last_ element of the queue
    ///
    /// NOTE: The head does not point to TaskRef but rather points to
    /// the TaskHeader despite being unintutive is because `enqueue` operation
    /// takes the ownership of the TaskRef and we cannot have a pointer to
    /// something that will go out of scope.
    head: UnsafeCell<*mut TaskHeader>,
}

impl TaskQueue {
    pub const fn new() -> Self {
        Self {
            head: UnsafeCell::new(null_mut()),
        }
    }

    /// enqueue enqueues a TaskRef into the queue.
    ///
    /// # Safety
    /// The caller must ensure that the task is not already in the queue.
    pub unsafe fn enqueue(&self, task: TaskRef) {
        let prev = NonNull::new(self.head.get().replace(task.as_ptr() as *mut _))
            .map(|ptr| TaskRef::from_ptr(ptr.as_ptr()));

        task.header().executor_queue_item.next.get().replace(prev);
    }

    /// drain drains the entire queue and calls `on_task` for each task that
    /// was in the queue. Drain will process the items in the reverse order of
    /// which they were enqueued.
    ///
    /// drain also resets the queue to the initial state hence the queue can be
    /// reused after calling drain.
    pub fn drain(&self, on_task: impl Fn(TaskRef)) {
        let curr = unsafe { self.head.get().replace(null_mut()) };

        let mut next = unsafe { NonNull::new(curr).map(|ptr| TaskRef::from_ptr(ptr.as_ptr())) };

        while let Some(task) = next {
            if let Some(task) = unsafe {
                replace(
                    &mut next,
                    task.header().executor_queue_item.next.get().replace(None),
                )
            } {
                on_task(task);
            }
        }
    }
}

/// TaskFreeListEmbedItem should be embedded into any struct that needs to be
/// enqueued into the tasks free list.
pub(crate) struct TaskFreeListEmbedItem {
    next: UnsafeCell<Option<TaskRef>>,
}
impl TaskFreeListEmbedItem {
    pub const fn new() -> Self {
        Self {
            next: UnsafeCell::new(None),
        }
    }
}

/// TaskFreeList is similar to [TaskFreeList]
pub struct TaskFreeList {
    head: UnsafeCell<*mut TaskHeader>,
}

impl TaskFreeList {
    pub const fn new() -> Self {
        Self {
            head: UnsafeCell::new(null_mut()),
        }
    }

    /// # Safety
    /// The caller must ensure that the task is not already in the queue.
    pub unsafe fn enqueue(&self, task: TaskRef) {
        let prev = NonNull::new(self.head.get().replace(task.as_ptr() as *mut _))
            .map(|ptr| TaskRef::from_ptr(ptr.as_ptr()));

        task.header().task_pool_queue_item.next.get().replace(prev);
    }

    /// # Safety
    /// The caller must ensure that the TaskRef's headers are properly initialized
    pub unsafe fn dequeue(&self) -> Option<TaskRef> {
        // # Safety
        // For as long as it is ensured that the enqueue method is called only with valid TaskRef
        // and they are not in the queue already.
        // Also, dereferencing the pointer should be OK as well as the map will be invoked iff
        // head is valid.
        let head = NonNull::new(self.head.get()).map(|head| TaskRef::from_ptr(*head.as_ptr()));

        if let Some(head) = &head {
            // # Safety
            // `next` will always be valid, it can be `None` but it will always be valid so dereferencing
            // should be safe
            let mut newhead = *head.header().task_pool_queue_item.next.get();
            if let Some(newhead) = &mut newhead {
                self.head.get().replace(newhead.mut_header() as *mut _);
            } else {
                self.head.get().replace(null_mut());
            }
        }

        head
    }
}
