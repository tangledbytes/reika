pub mod fs {
    use crate::iouring;

    use std::ffi::CString;
    use std::future::Future;
    use std::marker::PhantomData;
    use std::os::fd::RawFd;
    use std::task::Poll;

    pub struct Read<'a> {
        reactor: &'static iouring::Reactor,
        req: iouring::Request,

        phantom: PhantomData<&'a ()>,
    }

    impl<'a> Read<'a> {
        pub fn new(reactor: &'static iouring::Reactor, fd: RawFd, buf: &'a mut [u8]) -> Self {
            let read_op = io_uring::opcode::Read::new(
                io_uring::types::Fd(fd),
                buf.as_ptr() as *mut _,
                buf.len() as u32,
            );

            let req = iouring::Request::new(read_op.build());
            Self {
                reactor,
                req,
                phantom: PhantomData {},
            }
        }
    }

    impl<'a> Future for Read<'a> {
        type Output = i32;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            ctx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            if let Some(return_val) = self.req.return_val {
                return Poll::Ready(return_val);
            }

            self.req.waker = Some(ctx.waker().clone());

            unsafe {
                if self.reactor.submit(&mut self.req).is_err() {
                    // enqueue immediately
                    ctx.waker().wake_by_ref();
                }
            }

            Poll::Pending
        }
    }

    pub struct Open {
        reactor: &'static iouring::Reactor,
        req: iouring::Request,
        path: CString,
    }

    impl Open {
        pub fn new(reactor: &'static iouring::Reactor, pathname: &str) -> Self {
            let path = CString::new(pathname).expect("pathname should not contain null bytes");

            let open_op =
                io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), path.as_ptr());
            let req = iouring::Request::new(open_op.build());

            Self { reactor, req, path }
        }
    }

    impl Future for Open {
        type Output = RawFd;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            ctx: &mut std::task::Context<'_>,
        ) -> Poll<Self::Output> {
            if let Some(return_val) = self.req.return_val {
                return Poll::Ready(RawFd::from(return_val));
            }

            self.req.waker = Some(ctx.waker().clone());

            unsafe {
                if self.reactor.submit(&mut self.req).is_err() {
                    // enqueue immediately
                    ctx.waker().wake_by_ref();
                }
            }

            Poll::Pending
        }
    }

    pub struct Close {
        reactor: &'static iouring::Reactor,
        req: iouring::Request,
    }

    impl Close {
        pub fn new(reactor: &'static iouring::Reactor, fd: RawFd) -> Self {
            let open_op = io_uring::opcode::Close::new(io_uring::types::Fd(fd));
            let req = iouring::Request::new(open_op.build());

            Self { reactor, req }
        }
    }

    impl Future for Close {
        type Output = i32;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            ctx: &mut std::task::Context<'_>,
        ) -> Poll<Self::Output> {
            if let Some(return_val) = self.req.return_val {
                return Poll::Ready(return_val);
            }

            self.req.waker = Some(ctx.waker().clone());

            unsafe {
                if self.reactor.submit(&mut self.req).is_err() {
                    // enqueue immediately
                    ctx.waker().wake_by_ref();
                }
            }

            Poll::Pending
        }
    }
}
