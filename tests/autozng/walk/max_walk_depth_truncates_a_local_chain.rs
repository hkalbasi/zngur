//@ autozng
#![allow(dead_code)]

#[cfg_attr(autozng, autozng::emit)]
pub(crate) struct Root {
    pub child: Child1,
}

pub(crate) struct Child1 {
    pub child: Child2,
}

pub(crate) struct Child2 {
    pub child: Child3,
}

pub(crate) struct Child3 {
    pub child: Child4,
}

pub(crate) struct Child4 {
    pub child: Child5,
}

pub(crate) struct Child5 {
    pub child: Child6,
}

pub(crate) struct Child6 {
    pub child: Child7,
}

pub(crate) struct Child7 {
    pub child: Child8,
}

pub(crate) enum Child8 {
    Unit,
    Tuple(u32),
    Struct { value: u32 },
}
