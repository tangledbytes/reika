use std::{ io as stdio, os::fd::RawFd};

use libc::mode_t;

#[derive(Clone, Copy)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,

    custom_flags: i32,
    mode: mode_t,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,

            custom_flags: 0,
            mode: 0o666,
        }
    }

    pub fn read(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    pub fn write(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    pub fn append(&mut self, append: bool) -> &mut Self {
        self.append = append;
        self
    }

    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    pub fn mode(&mut self, mode: u32) -> &mut Self {
        self.mode = mode;
        self
    }

    pub fn flags(&mut self, flags: i32) -> &mut Self {
        self.custom_flags = flags;
        self
    }

    pub async fn open(&self, pathname: &str) -> stdio::Result<File> {
        let mut flags: i32 = 0;
        if self.append {
            flags |= libc::O_APPEND;
        }
        if self.truncate {
            flags |= libc::O_TRUNC;
        }
        if self.create {
            flags |= libc::O_CREAT;
        }
        if self.read && !self.write {
            flags |= libc::O_RDONLY;
        }
        if !self.read && self.write {
            flags |= libc::O_WRONLY;
        }
        if self.read && self.write {
            flags |= libc::O_RDWR;
        }

        flags |= self.custom_flags;

        let fd = raw::open(pathname, flags, self.mode).await?;

        Ok(File { fd })
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub struct File {
    fd: i32,
}

impl File {
    pub async fn open(pathname: &str) -> stdio::Result<File> {
        OpenOptions::new().open(pathname).await
    }

    pub async fn create(pathname: &str) -> stdio::Result<File> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .open(pathname)
            .await
    }

    pub fn options() -> OpenOptions {
        OpenOptions::default()
    }

    pub async fn sync_all(&self) -> stdio::Result<()> {
        let _ = raw::fsync(self.fd).await?;
        Ok(())
    }

    pub async fn sync_data(&self) -> stdio::Result<()> {
        let _ = raw::fdatasync(self.fd).await?;
        Ok(())
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd
    }

    pub async fn fallocate(&self, offset: u64, len: u64, mode: i32) -> stdio::Result<()> {
        let _ = raw::fallocate(self.fd, offset, len, mode).await?;
        Ok(())
    }

    pub async fn read(&self, buf: &'_ mut [u8]) -> stdio::Result<usize> {
        let n = raw::read(self.fd, buf).await?;
        Ok(n as usize)
    }

    pub async fn read_at(&self, buf: &'_ mut [u8], offset: u64) -> stdio::Result<usize> {
        let n = raw::read_at(self.fd, buf, offset as _).await?;
        Ok(n as usize)
    }

    pub async fn write(&self, buf: &'_ mut [u8]) -> stdio::Result<usize> {
        let n = raw::write(self.fd, buf).await?;
        Ok(n as usize)
    }

    pub async fn write_at(&self, buf: &'_ mut [u8], offset: u64) -> stdio::Result<usize> {
        let n = raw::write_at(self.fd, buf, offset as _).await?;
        Ok(n as usize)
    }

    pub async fn close(&self) -> stdio::Result<()> {
        let _ = raw::close(self.fd).await?;
        Ok(())
    }
}

pub mod raw {
    use crate::{PerThreadReactor, Reactor, ReactorRequest};
    use std::{ffi::CString, marker::PhantomData, os::fd::RawFd};

    #[derive(reika_macros::Future)]
    pub struct ReadMeta<'a> {
        reactor: &'static Reactor,
        req: ReactorRequest,

        phantom: PhantomData<&'a ()>,
    }

    pub fn read(fd: RawFd, buf: &'_ mut [u8]) -> ReadMeta<'_> {
        let reactor = unsafe { PerThreadReactor::this() };

        let read_op = io_uring::opcode::Read::new(
            io_uring::types::Fd(fd),
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
        )
        // Kernel will cast this to loff_t which is signed => -1
        .offset(u64::MAX);

        let req = ReactorRequest::new(read_op.build());
        ReadMeta {
            reactor,
            req,
            phantom: PhantomData {},
        }
    }

    pub fn read_at(fd: RawFd, buf: &'_ mut [u8], offset: i64) -> ReadMeta<'_> {
        let reactor = unsafe { PerThreadReactor::this() };

        let read_op = io_uring::opcode::Read::new(
            io_uring::types::Fd(fd),
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
        )
        .offset(offset.try_into().unwrap());

        let req = ReactorRequest::new(read_op.build());
        ReadMeta {
            reactor,
            req,
            phantom: PhantomData {},
        }
    }

    #[derive(reika_macros::Future)]
    pub struct OpenMeta {
        reactor: &'static Reactor,
        req: ReactorRequest,
        path: CString,
    }

    pub fn open(pathname: &str, flags: i32, mode: u32) -> OpenMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let path = CString::new(pathname).expect("pathname should not contain null bytes");

        let open_op =
            io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), path.as_ptr())
                .flags(flags)
                .mode(mode);

        let req = ReactorRequest::new(open_op.build());

        OpenMeta { reactor, req, path }
    }

    #[derive(reika_macros::Future)]
    pub struct CloseMeta {
        reactor: &'static Reactor,
        req: ReactorRequest,
    }

    pub fn close(fd: RawFd) -> CloseMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let close_op = io_uring::opcode::Close::new(io_uring::types::Fd(fd));
        let req = ReactorRequest::new(close_op.build());

        CloseMeta { reactor, req }
    }

    #[derive(reika_macros::Future)]
    pub struct WriteMeta<'a> {
        reactor: &'static Reactor,
        req: ReactorRequest,

        phantom: PhantomData<&'a ()>,
    }

    pub fn write(fd: RawFd, buf: &'_ mut [u8]) -> WriteMeta<'_> {
        let reactor = unsafe { PerThreadReactor::this() };

        let write_op = io_uring::opcode::Write::new(
            io_uring::types::Fd(fd),
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
        )
        // Kernel will cast this to loff_t which is signed => -1
        .offset(u64::MAX);

        let req = ReactorRequest::new(write_op.build());
        WriteMeta {
            reactor,
            req,
            phantom: PhantomData {},
        }
    }

    pub fn write_at(fd: RawFd, buf: &'_ mut [u8], offset: i64) -> WriteMeta<'_> {
        let reactor = unsafe { PerThreadReactor::this() };

        let write_op = io_uring::opcode::Write::new(
            io_uring::types::Fd(fd),
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
        )
        .offset(offset.try_into().unwrap());

        let req = ReactorRequest::new(write_op.build());
        WriteMeta {
            reactor,
            req,
            phantom: PhantomData {},
        }
    }

    #[derive(reika_macros::Future)]
    pub struct FsyncMeta {
        reactor: &'static Reactor,
        req: ReactorRequest,
    }

    pub fn fsync(fd: RawFd) -> FsyncMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let fsync_op = io_uring::opcode::Fsync::new(io_uring::types::Fd(fd));

        let req = ReactorRequest::new(fsync_op.build());
        FsyncMeta { reactor, req }
    }

    #[derive(reika_macros::Future)]
    pub struct FDatasyncMeta {
        reactor: &'static Reactor,
        req: ReactorRequest,
    }

    pub fn fdatasync(fd: RawFd) -> FDatasyncMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let fdatasync_op = io_uring::opcode::Fsync::new(io_uring::types::Fd(fd))
            .flags(io_uring::types::FsyncFlags::DATASYNC);

        let req = ReactorRequest::new(fdatasync_op.build());
        FDatasyncMeta { reactor, req }
    }

    #[derive(reika_macros::Future)]
    pub struct FallocateMeta {
        reactor: &'static Reactor,
        req: ReactorRequest,
    }

    pub fn fallocate(fd: RawFd, offset: u64, len: u64, mode: i32) -> FallocateMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let fallocate_op = io_uring::opcode::Fallocate::new(io_uring::types::Fd(fd), len)
            .offset(offset)
            .mode(mode);

        let req = ReactorRequest::new(fallocate_op.build());
        FallocateMeta { reactor, req }
    }
}
