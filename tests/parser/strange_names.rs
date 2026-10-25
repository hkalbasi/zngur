#[rustfmt::skip]
mod generated;

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct ____foo_ {
    pub ____field_: i32,
}

#[allow(non_camel_case_types)]
pub struct _Ab12cc {
    pub value: i32,
}

#[allow(non_camel_case_types)]
#[repr(C)]
pub enum _Enum12 {
    ____Variant_ { _payload12: i32 },
    _V2cc,
}

fn main() {
    assert_eq!(generated::____foo_fn_(), 42);
    assert_eq!(generated::_Fn12cc(1), 43);
    assert_eq!(generated::_verify_names_in_cpp(), 1);
}
