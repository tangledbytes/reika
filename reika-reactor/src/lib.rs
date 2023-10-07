#![cfg(target_os = "linux")]
pub mod error;
mod ops;
pub use ops::*;

extern crate libc;

use io_uring::{squeue, IoUring};
use std::{cell::UnsafeCell, io as stdio, task::Waker};

pub struct PerThreadReactor;

impl PerThreadReactor {
    thread_local! {
        static REACTOR: stdio::Result<Reactor> = Reactor::new(512);
    }

    /// this returns a static reference to the reactor
    /// (for current thread).
    ///
    /// # Safety
    /// The consumer of the function needs to ensure that the returned reference
    /// does NOT outlive the thread (that is, it should not be sent to other threads!)
    pub(crate) unsafe fn this() -> &'static Reactor {
        Self::REACTOR.with(|reactor_res: &stdio::Result<Reactor>| {
            let reactor = reactor_res.as_ref().unwrap();

            _make_static(reactor)
        })
    }

    /// submit takes a reference to request and submits the squeue entry part of it to
    /// the underlying IO Ring.
    ///
    /// # Safety
    /// It needs to be ensured the the [Request] and the data referred by the request lives
    /// at least for as long as the request is in the queue.
    pub(crate) unsafe fn submit(req: &mut ReactorRequest) -> stdio::Result<()> {
        let reactor = Self::this();
        reactor.submit(req)
    }

    pub fn flush(want: usize, timeouts: usize, etime: bool) -> stdio::Result<(usize, bool)> {
        let reactor = unsafe { Self::this() };
        reactor.flush(want, timeouts, etime)
    }

    pub fn run(ns: u32) -> stdio::Result<()> {
        let reactor = unsafe { Self::this() };
        reactor.run(ns)
    }

    pub fn run_for_ns(ns: u32) -> stdio::Result<()> {
        let reactor = unsafe { Self::this() };
        reactor.run_for_ns(ns)
    }
}

pub struct Reactor {
    ring: UnsafeCell<IoUring>,
    req_queued: UnsafeCell<usize>,
}

pub struct ReactorRequest {
    pub(crate) sentry: squeue::Entry,
    pub(crate) return_val: Option<i32>,
    pub(crate) waker: Option<Waker>,
}

impl ReactorRequest {
    pub fn new(sentry: squeue::Entry) -> Self {
        Self {
            sentry,
            return_val: None,
            waker: None,
        }
    }
}

impl Reactor {
    pub fn new(entries: u32) -> stdio::Result<Self> {
        let ring: io_uring::IoUring<io_uring::squeue::Entry, io_uring::cqueue::Entry> =
            IoUring::builder()
                .setup_coop_taskrun()
                .setup_single_issuer()
                .build(entries)?;
        Ok(Self {
            ring: UnsafeCell::new(ring),
            req_queued: UnsafeCell::new(0),
        })
    }

    /// submit takes a reference to request and submits the squeue entry part of it to
    /// the underlying IO Ring.
    ///
    /// # Safety
    /// It needs to be ensured the the [Request] and the data referred by the request lives
    /// at least for as long as the request is in the queue.
    pub unsafe fn submit(&'static self, req: &mut ReactorRequest) -> stdio::Result<()> {
        let mutreq = self.req_queued.get().as_mut().unwrap();
        *mutreq += 1;

        let mutring = self.ring.get().as_mut().unwrap();

        req.sentry = req.sentry.clone().user_data(req as *mut _ as u64);

        mutring
            .submission()
            .push(&req.sentry)
            .map_err(|_| stdio::Error::new(stdio::ErrorKind::Other, "failed to submit IO"))?;

        Ok(())
    }

    pub fn flush(&self, want: usize, timeouts: usize, etime: bool) -> stdio::Result<(usize, bool)> {
        self.flush_submissions(want, timeouts, etime)?;
        self.flush_completions(0, timeouts, etime)
    }
    pub fn run(&self, ns: u32) -> stdio::Result<()> {
        self.flush(0, 0, false);

        if !self.requires_reaping() {
            self.run_for_ns(ns)
        } else {
            Ok(())
        }
    }

    pub fn run_for_ns(&self, ns: u32) -> stdio::Result<()> {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };

        unsafe {
            let res = libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts as *mut _);
            assert_eq!(res, 0);
        }

        let mut timeouts: usize = 0;
        let mut etime = false;

        while !etime {
            let timeout_ts = io_uring::types::Timespec::new();
            timeout_ts.sec(ts.tv_sec as u64);
            timeout_ts.nsec(ts.tv_nsec as u32 + ns);

            let timeout_op = io_uring::opcode::Timeout::new(&timeout_ts as *const _).build();
            let timeout_op = timeout_op.user_data(0);
            timeouts += 1; // indicates submitting a timeout op

            unsafe {
                let mutself = self.ring.get().as_mut().unwrap();

                if mutself.submission().push(&timeout_op).is_err() {
                    (timeouts, etime) = self.flush_submissions(0, timeouts, etime)?;

                    // Try again, and crash if fails again
                    mutself.submission().push(&timeout_op).unwrap();
                }
            };

            (timeouts, etime) = self.flush(1, timeouts, etime)?;
        }

        while timeouts > 0 {
            (timeouts, etime) = self.flush_completions(0, timeouts, etime)?;
        }

        Ok(())
    }

    fn requires_reaping(&self) -> bool {
        let mutreq = unsafe { self.req_queued.get().as_mut().unwrap() };

        *mutreq > 0
    }

    fn flush_submissions(
        &self,
        want: usize,
        timeouts: usize,
        etime: bool,
    ) -> stdio::Result<(usize, bool)> {
        let mut timeouts = timeouts;
        let mut etime = etime;

        let mutself = unsafe { self.ring.get().as_mut().unwrap() };

        loop {
            if let Err(err) = mutself.submit_and_wait(want) {
                match err.raw_os_error() {
                    Some(libc::EINTR) => {
                        continue;
                    }
                    Some(libc::EBUSY) | Some(libc::EAGAIN) => {
                        (timeouts, etime) = self.flush_completions(1, timeouts, etime)?;
                        continue;
                    }
                    _ => {
                        return Err(err);
                    }
                }
            }

            return Ok((timeouts, etime));
        }
    }

    fn flush_completions(
        &self,
        want: usize,
        timeouts: usize,
        etime: bool,
    ) -> stdio::Result<(usize, bool)> {
        let mut collected = 0;
        let mut timeouts = timeouts;
        let mut etime = etime;

        let mutreq = unsafe { self.req_queued.get().as_mut().unwrap() };
        let mutself = unsafe { self.ring.get().as_mut().unwrap() };

        loop {
            for cqe in mutself.completion() {
                let udata = cqe.user_data();
                if udata == 0 {
                    timeouts -= 1;
                    if -cqe.result() == libc::ETIME {
                        etime = true;
                    }
                } else {
                    unsafe {
                        let req = udata as *mut ReactorRequest;
                        req.as_mut().unwrap().return_val = Some(cqe.result());
                        req.as_ref().unwrap().waker.as_ref().unwrap().wake_by_ref();
                    }
                    collected += 1;
                }
            }

            *mutreq -= collected;

            // Keep looping till we collect at least `want` completions
            if collected >= want {
                return Ok((timeouts, etime));
            }
        }
    }
}

unsafe fn _make_static<T>(i: &T) -> &'static T {
    std::mem::transmute(i)
}
