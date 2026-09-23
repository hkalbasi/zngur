#[rustfmt::skip]
mod generated;

#[allow(unused)]
#[derive(Debug)]
struct Foo {
    field1: i32,
    field2: String,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
struct FieldTypeA {
    pub fizz: FieldTypeC,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
// heap allocated
struct FieldTypeB {
    pub fizz: FieldTypeC,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
// auto field offset
struct FieldTypeC {
    pub buzz_1: i32,
    pub buzz_2: i32,
    pub buzz_3: i32,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
struct TypeA {
    pub foo: i32,
    pub bar: FieldTypeA,
    pub baz: FieldTypeB,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
// heap allocated
struct TypeB {
    pub foo: i32,
    pub bar: FieldTypeA,
    pub baz: FieldTypeB,
}

#[allow(unused)]
#[derive(Debug)]
/// auto field offset + layout_conservative
struct ConservativeLayoutType {
    pub field1: f32,
    pub field2: i32,
    pub field3: String,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
/// bool field + Copy struct field, to test Ref/RefMut/FieldOwned<T> conversion on
/// specialized (non-primitive) field types
struct BoolHolder {
    pub flag: bool,
    pub data: FieldTypeC,
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
/// Option fields, to test Field* conversion for Copy generic instantiations
/// (`Option<&T>` and `Option<T: Copy>` are both `Copy`)
struct OptionHolder<'a> {
    pub opt_ref: Option<&'a i32>,
    pub opt_copy: Option<FieldTypeC>,
}

fn make_option_holder() -> OptionHolder<'static> {
    let leaked: &'static i32 = Box::leak(Box::new(1234));
    OptionHolder {
        opt_ref: Some(leaked),
        opt_copy: Some(FieldTypeC {
            buzz_1: 11,
            buzz_2: 22,
            buzz_3: 33,
        }),
    }
}

fn make_empty_option_holder() -> OptionHolder<'static> {
    OptionHolder {
        opt_ref: None,
        opt_copy: None,
    }
}

#[allow(unused)]
impl ConservativeLayoutType {
    pub fn mem_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
    pub fn mem_align(&self) -> usize {
        std::mem::align_of::<Self>()
    }
}

struct Scoped(&'static str);

impl Scoped {
    fn new(message: &'static str) -> Self {
        println!("{message} -- started");
        Self(message)
    }
}

impl Drop for Scoped {
    fn drop(&mut self) {
        println!("{} -- finished", self.0);
        println!();
    }
}

fn call_dyn_fn_multi_args(func: Box<dyn Fn(i32, crate::Scoped, &str)>) {
    let scope = Scoped::new("scope passed to dyn Fn");
    func(2, scope, "hello");
    println!("End of call_dyn_fn_multi_args");
}

#[derive(Debug)]
pub struct SimpleFoo(pub i32);

#[derive(Debug)]
pub struct RefHolder<'a>(pub &'a SimpleFoo);

#[derive(Debug)]
struct ZeroSizedType;

impl ZeroSizedType {
    fn new() -> Self {
        Self
    }

    fn method(&self) {
        println!("Method call on ZST");
    }
}
