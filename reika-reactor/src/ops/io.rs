use std::{ffi::CString, marker::PhantomData, os::fd::RawFd};
pub use libc;

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
    )
    // Kernel will cast this to loff_t which is signed => -1
    .offset(u64::MAX);

    let req = iouring::Request::new(read_op.build());
    ReadMeta {
        reactor,
        req,
        phantom: PhantomData {},
    }
}

pub fn read_at(fd: RawFd, buf: &'_ mut [u8], offset: i64) -> ReadMeta<'_> {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let read_op = io_uring::opcode::Read::new(
        io_uring::types::Fd(fd),
        buf.as_mut_ptr() as *mut _,
        buf.len() as u32,
    )
    .offset(offset.try_into().unwrap());

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

pub fn open(pathname: &str, flags: i32, mode: u32) -> OpenMeta {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let path = CString::new(pathname).expect("pathname should not contain null bytes");

    let open_op = io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), path.as_ptr())
        .flags(flags).mode(mode);

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

    let close_op = io_uring::opcode::Close::new(io_uring::types::Fd(fd));
    let req = iouring::Request::new(close_op.build());

    CloseMeta { reactor, req }
}

#[derive(reika_macros::Future)]
pub struct WriteMeta<'a> {
    reactor: &'static iouring::Reactor,
    req: iouring::Request,

    phantom: PhantomData<&'a ()>,
}

pub fn write(fd: RawFd, buf: &'_ mut [u8]) -> WriteMeta<'_> {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let write_op = io_uring::opcode::Write::new(
        io_uring::types::Fd(fd),
        buf.as_mut_ptr() as *mut _,
        buf.len() as u32,
    )
    // Kernel will cast this to loff_t which is signed => -1
    .offset(u64::MAX);

    let req = iouring::Request::new(write_op.build());
    WriteMeta {
        reactor,
        req,
        phantom: PhantomData {},
    }
}

pub fn write_at(fd: RawFd, buf: &'_ mut [u8], offset: i64) -> WriteMeta<'_> {
    let reactor = unsafe { iouring::Reactor::get_static() };

    let write_op = io_uring::opcode::Write::new(
        io_uring::types::Fd(fd),
        buf.as_mut_ptr() as *mut _,
        buf.len() as u32,
    )
    .offset(offset.try_into().unwrap());

    let req = iouring::Request::new(write_op.build());
    WriteMeta {
        reactor,
        req,
        phantom: PhantomData {},
    }
}