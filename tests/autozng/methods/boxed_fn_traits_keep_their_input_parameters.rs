//@ autozng

pub struct Callbacks;

impl Callbacks {
    pub fn set_once(&mut self, cb: Box<dyn FnOnce(u32) -> bool>) {
        let _ = cb;
    }

    pub fn set_mut(&mut self, cb: Box<dyn FnMut(&str) -> bool>) {
        let _ = cb;
    }

    pub fn set_ref(&mut self, cb: Box<dyn Fn()>) {
        let _ = cb;
    }
}
