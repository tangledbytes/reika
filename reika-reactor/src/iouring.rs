#![cfg(target_os = "linux")]

extern crate libc;

use io_uring::{squeue, IoUring};
use std::{
    cell::UnsafeCell,
    io,
    task::{Context, Waker},
};

use crate::error::{InitFail, RequestError};

pub struct Reactor {
    ring: UnsafeCell<IoUring>,
}

pub struct Request {
    pub(crate) sentry: squeue::Entry,
    pub(crate) return_val: Option<i32>,
    pub(crate) waker: Option<Waker>,
}

impl Request {
    pub fn new(sentry: squeue::Entry) -> Self {
        Self {
            sentry,
            return_val: None,
            waker: None,
        }
    }
}

impl Reactor {
    pub fn new(entries: u32) -> Result<Self, InitFail> {
        let ring =
            IoUring::new(entries).map_err(|_| InitFail::new("failed to initialize the ring"))?;
        Ok(Self {
            ring: UnsafeCell::new(ring),
        })
    }

    pub unsafe fn submit(&'static self, req: &mut Request) -> Result<(), RequestError> {
        let mutself = self.ring.get().as_mut().unwrap();

        req.sentry = req.sentry.clone().user_data(req as *mut _ as u64);

        mutself
            .submission()
            .push(&req.sentry)
            .map_err(|_| RequestError::Push)?;

        Ok(())
    }

    pub fn flush(&self, want: usize, timeouts: usize, etime: bool) -> io::Result<(usize, bool)> {
        self.flush_submissions(want, timeouts, etime)?;
        self.flush_completions(0, timeouts, etime)
    }

    pub fn run_for_ns(&self, ns: u32) -> io::Result<()> {
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

    fn flush_submissions(
        &self,
        want: usize,
        timeouts: usize,
        etime: bool,
    ) -> io::Result<(usize, bool)> {
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
    ) -> io::Result<(usize, bool)> {
        let mut collected = 0;
        let mut timeouts = timeouts;
        let mut etime = etime;

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
                        let req = udata as *mut Request;
                        req.as_mut().unwrap().return_val = Some(cqe.result());
                        req.as_ref().unwrap().waker.as_ref().unwrap().wake_by_ref();
                    }
                    collected += 1;
                }
            }

            // Keep looping till we collect at least `want` completions
            if collected >= want {
                return Ok((timeouts, etime));
            }
        }
    }
}
