use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub struct InitFail {
    msg: &'static str,
}
impl InitFail {
    pub fn new(msg: &'static str) -> Self {
        Self { msg }
    }
}
impl Error for InitFail {}
impl Display for InitFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to initialize the reactor: {}", self.msg)
    }
}

#[derive(Debug)]
pub enum RequestError {
    Push,
}
impl Error for RequestError {}
impl Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Push => write!(f, "failed to submit request to IO Uring"),
        }
    }
}
