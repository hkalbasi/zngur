//! Inherent methods: receivers, static methods, renaming and unsupported signatures.

use expect_test::expect;

use crate::harness::run;

#[test]
fn receivers_and_static_methods() {
    let out = run(r#"
pub struct Counter {
    pub value: i32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { value: 0 }
    }

    pub fn get(&self) -> i32 {
        self.value
    }

    pub fn set(&mut self, value: i32) {
        self.value = value;
    }

    pub fn consume(self) -> i32 {
        self.value
    }

    pub fn add(a: i32, b: i32) -> i32 {
        a + b
    }
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Counter {
            #layout(size = 4, align = 4);

            field value (offset = 0, type = i32);

            fn new() -> crate::Counter;
            fn get(&self) -> i32;
            fn set(&mut self, i32);
            fn consume(self) -> i32;
            fn add(i32, i32) -> i32;
        }
    "#]]);
}

#[test]
fn rename_a_method() {
    let out = run(r#"
pub struct Thing;

impl Thing {
    #[autozng::rename("Renamed")]
    pub fn original(&self) {}

    #[autozng::rename(case = "Pascal")]
    pub fn long_snake_name(&self) {}
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Thing {
            #layout(size = 0, align = 1);

            fn original(&self) as Renamed;
            fn long_snake_name(&self) as LongSnakeName;
        }
    "#]]);
}

#[test]
fn unsupported_receiver_is_skipped() {
    let out = run(r#"
pub struct OnlyBoxable;

impl OnlyBoxable {
    pub fn by_box(self: Box<Self>) {}
    pub fn fine(&self) {}
}
"#);
    out.assert_zng(expect![[r#"
        type crate::OnlyBoxable {
            #layout(size = 0, align = 1);

            fn fine(&self);
        }
    "#]]);
}

#[test]
fn unsupported_argument_is_skipped() {
    let out = run(r#"
pub struct Args;

impl Args {
    pub fn ok(&self) {}

    pub fn takes_array(&self, values: [u8; 4]) {
        let _ = values;
    }
}
"#);
    out.assert_zng(expect![[r#"
        type crate::Args {
            #layout(size = 0, align = 1);

            fn ok(&self);
        }
    "#]]);
}
