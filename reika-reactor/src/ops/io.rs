use std::{ffi::CString, marker::PhantomData, os::fd::RawFd};

use crate::iouring;

#[derive(reika_macros::Future)]
pub struct ReadMeta<'a> {
    reactor: &'static iouring::Reactor,
    req: iouring::Request,

    phantom: PhantomData<&'a ()>,
}

pub fn read(fd: RawFd, buf: &'_ mut [u8]) -> ReadMeta<'_> {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let read_op = io_uring::opcode::Read::new(
        io_uring::types::Fd(fd),
        buf.as_mut_ptr() as *mut _,
        buf.len() as u32,
    );

    let req = iouring::Request::new(read_op.build());
    ReadMeta {
        reactor,
        req,
        phantom: PhantomData {},
    }
}

#[derive(reika_macros::Future)]
pub struct OpenMeta {
    reactor: &'static iouring::Reactor,
    req: iouring::Request,
    path: CString,
}

pub fn open(pathname: &str, flags: i32) -> OpenMeta {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let path = CString::new(pathname).expect("pathname should not contain null bytes");

    let open_op = io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), path.as_ptr())
        .flags(flags);

    let req = iouring::Request::new(open_op.build());

    OpenMeta { reactor, req, path }
}

#[derive(reika_macros::Future)]
pub struct CloseMeta {
    reactor: &'static iouring::Reactor,
    req: iouring::Request,
}

pub fn close(fd: RawFd) -> CloseMeta {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let open_op = io_uring::opcode::Close::new(io_uring::types::Fd(fd));
    let req = iouring::Request::new(open_op.build());

    CloseMeta { reactor, req }
}
