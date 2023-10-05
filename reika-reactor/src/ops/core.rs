use crate::{Reactor, ReactorRequest, PerThreadReactor};

#[derive(reika_macros::Future)]
struct YieldMeta {
    reactor: &'static Reactor,
    req: ReactorRequest,
}

#[inline(always)]
pub async fn yield_now() {
	_yield_now().await.unwrap();
}

fn _yield_now() -> YieldMeta {
    let reactor = unsafe { PerThreadReactor::this() };

    let nop = io_uring::opcode::Nop::new();

    let req = ReactorRequest::new(nop.build());
    YieldMeta {
        reactor,
        req,
    }
}
