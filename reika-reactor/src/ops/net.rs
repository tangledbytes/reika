use std::io::{Error, Result};
use std::marker::PhantomData;
use std::mem::size_of;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::RawFd;

use crate::{Reactor, ReactorRequest, io, PerThreadReactor};

pub const SOMAXCONN: i32 = libc::SOMAXCONN;

#[derive(Clone, Copy)]
pub struct TcpListner {
    sock_fd: RawFd,
}

#[derive(Clone, Copy)]
pub struct TcpStream {
    connfd: RawFd,
}

#[derive(reika_macros::Future)]
pub struct TcpReadMeta<'a> {
    reactor: &'static Reactor,
    req: ReactorRequest,

    phantom: PhantomData<&'a ()>,
}

#[derive(reika_macros::Future)]
pub struct TcpWriteMeta<'a> {
    reactor: &'static Reactor,
    req: ReactorRequest,

    phantom: PhantomData<&'a ()>,
}

#[derive(reika_macros::Future)]
struct SocketMeta {
    reactor: &'static Reactor,
    req: ReactorRequest,
}

#[derive(reika_macros::Future)]
struct AcceptMeta {
    reactor: &'static Reactor,
    req: ReactorRequest,
}

impl TcpListner {
    pub async fn bind(addr: &str, backlog: i32) -> Result<TcpListner> {
        let parsed_addr: SocketAddr = addr
            .parse()
            .map_err(|err| Error::new(std::io::ErrorKind::InvalidData, err))?;

        let sock_fd = match parsed_addr {
            SocketAddr::V4(ref a) => {
                let socket =
                    Self::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0).await?;
                unsafe {
                    Self::_bind4(socket, a.ip(), a.port())?;
                }
                socket
            }
            SocketAddr::V6(ref a) => {
                let socket =
                    Self::socket(libc::AF_INET6, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0).await?;
                unsafe {
                    Self::_bind6(socket, a.ip(), a.port())?;
                }
                socket
            }
        };

        if sock_fd == 0 {
            return Err(Error::new(std::io::ErrorKind::Other, "failed to bind"));
        }

        unsafe {
            Self::defaultsockopt(sock_fd)?;
            Self::listen(sock_fd, backlog)?;
        }

        Ok(TcpListner { sock_fd })
    }

    #[inline(always)]
    pub async fn accept(&self) -> Result<TcpStream> {
        let fd = Self::_accept(self.sock_fd).await?;
        Ok(TcpStream { connfd: fd })
    }

    fn _accept(socket: RawFd) -> AcceptMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let accept_op = io_uring::opcode::Accept::new(
            io_uring::types::Fd(socket),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let req = ReactorRequest::new(accept_op.build());
        AcceptMeta { reactor, req }
    }

    fn socket(domain: i32, socket_type: i32, protocol: i32) -> SocketMeta {
        let reactor = unsafe { PerThreadReactor::this() };

        let socket_op = io_uring::opcode::Socket::new(domain, socket_type, protocol);
        let req = ReactorRequest::new(socket_op.build());
        SocketMeta { reactor, req }
    }

    unsafe fn listen(socket: RawFd, backlog: i32) -> Result<()> {
        let res = libc::listen(socket, backlog);
        if res == 0 {
            Ok(())
        } else {
            Err(Error::from_raw_os_error(-res))
        }
    }

    unsafe fn defaultsockopt(socket: RawFd) -> Result<()> {
        let yes = 1i32;
        let res = libc::setsockopt(
            socket,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &yes as *const _ as *const libc::c_void,
            size_of::<i32>() as _,
        );

        if res == 0 {
            Ok(())
        } else {
            Err(Error::from_raw_os_error(-res))
        }
    }

    unsafe fn _bind4(socket: libc::c_int, addr: &Ipv4Addr, port: u16) -> Result<()> {
        let sockaddr = libc::sockaddr_in {
            sin_family: libc::AF_INET as _,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from_be_bytes(addr.octets()).to_be(),
            },
            sin_zero: [0; 8],
        };

        let bindres = libc::bind(
            socket,
            &sockaddr as *const _ as *const libc::sockaddr,
            size_of::<libc::sockaddr_in>() as _,
        );

        if bindres == 0 {
            Ok(())
        } else {
            Err(Error::from_raw_os_error(-bindres))
        }
    }

    unsafe fn _bind6(_socket: libc::c_int, _addr: &Ipv6Addr, _port: u16) -> Result<()> {
        unimplemented!()
    }
}

impl TcpStream {
	#[inline(always)]
    pub async fn read(&self, buf: &'_ mut [u8]) -> Result<usize> {
        let readbytes = Self::_read(self.connfd, buf).await?;
        Ok(readbytes as usize)
    }

	#[inline(always)]
    pub async fn send(&mut self, buf: &'_ [u8]) -> Result<usize> {
        let sendbytes = Self::_write(self.connfd, buf).await?;
        Ok(sendbytes as usize)
    }

	#[inline(always)]
	pub async fn close(&mut self) -> Result<()> {
		let _ = io::raw::close(self.connfd).await?;
		Ok(())
	}

    fn _write(fd: RawFd, buf: &'_ [u8]) -> TcpWriteMeta<'_> {
        let reactor = unsafe { PerThreadReactor::this() };

        let send_op = io_uring::opcode::Send::new(
            io_uring::types::Fd(fd),
            buf.as_ptr() as *const _,
            buf.len() as u32,
        );

        let req = ReactorRequest::new(send_op.build());
        TcpWriteMeta {
            reactor,
            req,
            phantom: PhantomData {},
        }
    }

    fn _read(fd: RawFd, buf: &'_ mut [u8]) -> TcpReadMeta<'_> {
        let reactor = unsafe { PerThreadReactor::this() };

        let recv_op = io_uring::opcode::Recv::new(
            io_uring::types::Fd(fd),
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
        );

        let req = ReactorRequest::new(recv_op.build());
        TcpReadMeta {
            reactor,
            req,
            phantom: PhantomData {},
        }
    }
}
