#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]
#![cfg_attr(autozng, autozng::policy("test"))]
#![allow(dead_code)]
//! #[autozng::policy("test")]
//! #[autozng::policy("test_2")]

#![allow(unused_variables)]

use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

pub mod submod;

/// Docs for Example type
///   multiple lines
///     multiple attrs
///
/// <!--
/// #[autozng::rename(case = "bad2")]
/// -->
///
#[cfg_attr(autozng, autozng::emit)]
#[cfg_attr(autozng, autozng::emit(fifth))]
#[cfg_attr(autozng, autozng::emit(third, forth))]
#[cfg_attr(autozng, autozng::rename(case = "Ada"))]
#[derive(Default, Debug, Clone)]
pub struct Example {
    /// autozng::rename("inner")
    #[cfg_attr(autozng, autozng::rename("inner"))]
    pub first: String,
    second: i32,
    third: u32,
    forth: Vec<String>,
    fifth: Vec<u32>,
}

impl std::fmt::Display for Example {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r#""{}" + "{}""#, self.first, self.second)
    }
}

impl Example {
    #[cfg_attr(autozng, autozng::rename(case = "Pascal"))]
    pub fn a_long_fn_that_will_be_renamed(&self) {}
}

/// A free function: unlike an associated method it has no `cpp_name` slot in the IDL, so
/// `rename` is ignored (with a warning).
#[cfg_attr(autozng, autozng::rename(case = "Pascal"))]
pub fn a_free_fn_that_will_not_be_renamed(value: i32) -> i32 {
    value
}

/// Alias of Example
type Alias = Example;

#[derive(Debug, Copy, Clone)]
pub enum EnumEx {
    V1,
    V2(i32),
    V3(u32, u32),
    V4 { f1: u32 },
    V5 { f1: u32, f2: u32 },
}

pub struct Exotics {
    fn_def: Box<dyn FnOnce(i32) -> bool>,
    async_fn_def: Box<dyn Fn(i32) -> Box<dyn core::future::Future<Output = bool>> + Send + Sync>,
}

#[derive(Debug)]
pub struct GenericExample<T: std::fmt::Debug> {
    pub val: T,
}

impl<T: std::fmt::Debug> GenericExample<T> {
    pub fn get_debug_string(&self) -> String {
        format!("{self:?}")
    }
}

impl<T: Clone + std::fmt::Debug> GenericExample<T> {
    pub fn inner_clone(&self) -> T {
        self.val.clone()
    }

    pub fn inner_clone_in_tuple_with<D>(&self, d: D) -> (T, D) {
        (self.val.clone(), d)
    }

    pub fn inner_clone_array<const N: usize>(&self) -> [T; N] {
        std::iter::repeat_n(&self.val, N)
            .cloned()
            .collect::<Vec<T>>()
            .try_into()
            .unwrap()
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const MONO: () = {
        let _ = Self::inner_clone_in_tuple_with::<u8>;
        #[autozng::rename("inner_clone_in_tuple_with_vec")]
        let _ = Self::inner_clone_in_tuple_with::<Vec<u8>>;
        let _ = Self::inner_clone_array::<8>;
    };
}

#[derive(Debug)]
pub struct NonCloneType(pub u32);

#[cfg(autozng)]
#[autozng::emit_items]
const _: &[(
    GenericExample<NonCloneType>,
    GenericExample<u64>,
    GenericExample<Vec<u32>>,
)] = &[];

/// a Trait
///   multiline doc comment
pub trait MyTrait {
    type Output;
    fn trait_fn(&self) -> &str;
}

pub type ExBox = Box<Example>;

/// trait impl
impl MyTrait for Example {
    type Output = Example;
    fn trait_fn(&self) -> &str {
        self.by_ref("from_trait");
        "from_my_trait"
    }
}

/// main impl block
#[allow(unused)]
impl Example {
    pub fn new() -> Self {
        Example {
            first: "default".to_string(),
            second: 0,
            third: 0,
            forth: vec!["hello".into(), "world".into()],
            fifth: vec![0],
        }
    }

    fn private_hello(&self) -> &str {
        "hello"
    }

    pub fn static_add_one(a: i32) -> i32 {
        a + 1
    }

    pub fn second_in_tuple_with<D>(&self, d: D) -> (i32, D) {
        (self.second, d)
    }

    #[cfg(autozng)]
    #[autozng::emit_items]
    const __SECOND_IN_TUPLE_WITH_MONO_FOR: () = {
        let __second_in_tuple_with_mono_for_u8 = Self::second_in_tuple_with::<u8>;
        let __second_in_tuple_with_mono_for_u32 = Self::second_in_tuple_with::<u32>;
    };

    /// by value fn
    pub fn by_value(self) {
        self.by_ref("by_val");
    }

    /// by ref fn
    pub fn by_ref(&self, source: &str) {
        println!("{source}: {}", self.first);
    }

    /// by ref mut fn
    pub fn by_ref_mut(&mut self) {
        self.first = "by_ref_mut".to_string();
        self.by_ref("mut");
    }

    /// by box fn
    // Deliberately exercises a `Box<Self>` receiver (autozng's receiver handling).
    #[allow(clippy::boxed_local)]
    pub fn by_box(self: Box<Self>) {
        self.by_ref("by_box");
    }

    /// by rc fn
    pub fn by_rc(self: Rc<Self>) {
        self.by_ref("by_rc");
    }

    /// by arc fn
    pub fn by_arc(self: Arc<Self>) {
        self.by_ref("by_arc");
    }

    /// by pin fn
    pub fn by_pin(self: Pin<&Self>) {
        self.by_ref("by_pin");
    }

    /// by explicit type fn
    pub fn explicit_type(self: Arc<Example>) {
        self.by_ref("explicit");
    }

    /// fn with lifetime
    pub fn with_lifetime(&self) {
        self.by_ref("lifetime");
    }

    /// fn with nested type
    pub fn nested(self: &mut &Arc<Rc<Box<Alias>>>) {
        self.by_ref("nested");
    }

    /// fn with type by projection
    pub fn via_projection(self: <Example as MyTrait>::Output) {
        self.by_ref("via_projection");
    }

    /// from ctor
    pub fn from(name: String) -> Self {
        Example {
            first: name,
            second: -1,
            third: 1,
            forth: vec!["hello".into(), "world".into()],
            fifth: vec![1, 2, 3, 4],
        }
    }

    pub async fn do_async(&self) -> String {
        "a suposably async message".to_owned()
    }
}

#[cfg_attr(autozng, autozng::emit_on_implementers)]
#[cfg_attr(autozng, autozng::emit)]
trait YarrExt {
    fn yarrrr(&self) -> String;
}

impl<T> YarrExt for T
where
    T: MyTrait,
{
    fn yarrrr(&self) -> String {
        format!("yarrrr {}", self.trait_fn())
    }
}
#[cfg(autozng)]
#[autozng::emit_items]
const _: Option<(
    GenericExample<u8>,
    GenericExample<String>,
    GenericExample<i32>,
)> = None;

#[cfg(autozng)]
#[autozng::emit_items]
#[allow(unused_doc_comments)]
const _: () = {
    #[autozng::rename("in_tuple_with_u8")]
    let _ = Example::second_in_tuple_with::<u8>;
    ///  #[autozng::rename("in_tuple_with_u32")]
    let _ = Example::second_in_tuple_with::<u32>;
    let _ = Example::to_string;
    let _ = <Example as YarrExt>::yarrrr;
};

pub trait GenericTrait<T> {
    fn get_value(&self) -> T;
}

#[cfg(autozng)]
#[autozng::emit_items(emit_traits = true)]
const _: [(&dyn GenericTrait<Vec<u32>>, &dyn GenericTrait<u64>); 0] = [];

#[derive(Default)]
#[allow(clippy::type_complexity)]
pub struct TyWithCallbacks {
    cb1: Option<Box<dyn FnOnce() -> bool>>,
    cb2: Option<Box<dyn FnOnce(u32) -> bool>>,
    cb3: Option<Box<dyn FnMut(&str) -> bool>>,
    cb4: Option<Box<dyn Fn(&Self)>>,
}

impl TyWithCallbacks {
    pub fn set_cb1(&mut self, cb: Box<dyn FnOnce() -> bool>) {
        self.cb1 = Some(cb)
    }
    pub fn set_cb2(&mut self, cb: Box<dyn FnOnce(u32) -> bool>) {
        self.cb2 = Some(cb)
    }
    pub fn set_cb3(&mut self, cb: Box<dyn FnMut(&str) -> bool>) {
        self.cb3 = Some(cb)
    }
    pub fn set_cb4(&mut self, cb: Box<dyn Fn(&Self)>) {
        self.cb4 = Some(cb)
    }
}

/// main entry fn
fn main() {
    let example = Example::from("Hello".to_string());
    example.by_value();

    let boxed = Box::new(Example::default());
    boxed.by_box();

    Example::default().by_ref_mut();
    Example::default().with_lifetime();

    let exotics = Exotics {
        fn_def: Box::new(|a| a > 1),
        async_fn_def: Box::new(|a| {
            async fn fut(a: i32) -> bool {
                a < 2
            }
            Box::new(fut(a))
        }),
    };
}
