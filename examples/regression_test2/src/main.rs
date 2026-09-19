#![allow(non_camel_case_types)]

#[rustfmt::skip]
mod generated;

use std::future::Future;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use cpp_inherit::CppInherit;

use crate::generated::{CppTask, Dispatcher};

pub struct RustTask<'a>(pub Pin<&'a mut dyn Future<Output = ()>>);

impl<'a> RustTask<'a> {
    pub fn poll(&mut self) -> Poll<()> {
        let waker = unsafe { Waker::from_raw(dummy_raw_waker()) };
        let mut cx = Context::from_waker(&waker);
        self.0.as_mut().poll(&mut cx)
    }
}

unsafe fn dummy_raw_waker() -> std::task::RawWaker {
    static DUMMY_WAKER_VTABLE: std::task::RawWakerVTable =
        std::task::RawWakerVTable::new(|_| unsafe { dummy_raw_waker() }, |_| (), |_| (), |_| ());
    std::task::RawWaker::new(std::ptr::null(), &DUMMY_WAKER_VTABLE)
}

fn main() {
    let mut fut = async {
        println!("Hello from Rust async!");
    };

    let fut = RustTask(unsafe { Pin::new_unchecked(&mut fut) });
    let mut cpp_task: MaybeUninit<CppTask> = MaybeUninit::uninit();
    unsafe {
        cpp_task.assume_init_mut().constructor();
    }
    let mut task: CppInherit<CppTask, RustTask> =
        CppInherit::new(fut, unsafe { cpp_task.assume_init() });
    let mut dispatcher: MaybeUninit<Dispatcher> = MaybeUninit::uninit();
    unsafe {
        dispatcher.assume_init_mut().constructor();
    }
    let dispatcher = unsafe { dispatcher.assume_init() };
    dispatcher.run_task(&mut task.inner);
}
