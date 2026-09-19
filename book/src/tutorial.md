# Tutorial

A Zngur project consists of 3 things:

- An IDL (interface definition language) file named `main.zng`
- A Rust crate (that can be anything: binary, rlib, static-lib, cdy-lib, ...)
- A C++ project.

To start, install Zngur:

```
cargo install zngur-cli
```

Then generate a new `staticlib` crate using `cargo init --lib`
and by appending this to `Cargo.toml`:

```Toml
[lib]
crate-type = ["staticlib"]
```

And create an empty `main.cpp` and `main.zng` file.
Your directory tree should look like this:

```
├── Cargo.toml
├── main.cpp
├── main.zng
└── src
    └── lib.rs
```

## Basic structure of `main.zng`

Imagine we want to use this inventory in C++:

```Rust
struct Item {
    name: String,
    size: u32,
}

struct Inventory {
    items: Vec<Item>,
    remaining_space: u32,
}

impl Inventory {
    fn new_empty(space: u32) -> Self {
        Self {
            items: vec![],
            remaining_space: space,
        }
    }
}
```

Copy it into `src/lib.rs`.
Now we need to declare things that we need to access in C++ in the `main.zng` file:

```
type crate::Inventory {
    #layout(size = 32, align = 8);

    fn new_empty(u32) -> crate::Inventory;
}
```

Zngur needs to know the size and alignment of the types inside the bridge.
You can find these using rust-analyzer (by hovering over the struct or type alias)
or fill it with some random number and then fix it from the compiler error.

> **Note:** Ideally `main.zng` should be auto-generated, but we are not there yet.
> Also, Zngur can work without explicit size and alignment (with some caveats),
> see [layout policies](./call_rust_from_cpp/layout_policy.md) for more details.

Now, run `zngur g ./main.zng` to generate the C++ and Rust glue files.
It will generate a `./generated.h` C++ header file, and a `./src/generated.rs` file.
Add a `mod generated;` to your `lib.rs` file to include the generated Rust file.
Then fill `main.cpp` file with the following content:

```C++
#include "./generated.h"

int main() {
  auto inventory = rust::crate::Inventory::new_empty(1000);
}
```

Zngur will add every Rust item with its full path in the `rust` namespace,
so for example `String` will become `rust::std::string::String` in the C++ side.

To build it, you first need to build the Rust code using `cargo build`,
which will generate a `libyourcrate.a` in the `./target/debug` folder,
and then you can build your C++ code by linking to it:

```bash
clang++ main.cpp -g -L ./target/debug/ -l your_crate
```

To ensure that everything works,
let's add a `#[derive(Debug)]` to `Inventory` and use `zngur_dbg` to see it:

```
type crate::Inventory {
    #layout(size = 32, align = 8);
    wellknown_traits(Debug);

    fn new_empty(u32) -> crate::Inventory;
}
```

```C++
int main() {
  auto inventory = rust::crate::Inventory::new_empty(1000);
  zngur_dbg(inventory);
}
```

There are some traits that Zngur has special support for them, and `Debug` is among them.
[This page](./call_rust_from_cpp/wellknown_traits.md) has the complete list of them.

Assuming that everything works correctly,
you should see something like this after executing the program:

```
[main.cpp:5] inventory = Inventory {
    items: [],
    remaining_space: 1000,
}
```

Now let's add some more methods to it:

```Rust
impl Inventory {
    fn add_item(&mut self, item: Item) {
        self.remaining_space -= item.size;
        self.items.push(item);
    }

    fn add_banana(&mut self, count: u32) {
        for _ in 0..count {
            self.add_item(Item {
                name: "banana".to_owned(),
                size: 7,
            });
        }
    }
}
```

```
type crate::Inventory {
    #layout(size = 32, align = 8);
    wellknown_traits(Debug);

    fn new_empty(u32) -> crate::Inventory;
    fn add_banana(&mut self, u32);
}
```

Now we can use it in the C++ file:

```C++
#include "./generated.h"

int main() {
  auto inventory = rust::crate::Inventory::new_empty(1000);
  inventory.add_banana(3);
  zngur_dbg(inventory);
}
```

```
[main.cpp:6] inventory = Inventory {
    items: [
        Item {
            name: "banana",
            size: 7,
        },
        Item {
            name: "banana",
            size: 7,
        },
        Item {
            name: "banana",
            size: 7,
        },
    ],
    remaining_space: 979,
}
```

Bridging the `add_item` method requires a little more effort.
We need to declare `crate::Item` type as well since it is an argument of that function:

```
// ...

type crate::Item {
    #layout(size = 32, align = 8);
}

type crate::Inventory {
    // ...
    fn add_item(&mut self, crate::Item);
}
```

But using that alone we can't use the `add_item`
since there is no way to obtain a `rust::crate::Item` in the C++ side.
To fix that, we need to add the constructor for the `Item`:

```
type ::std::string::String {
    #layout(size = 24, align = 8);
}

type crate::Item {
    #layout(size = 32, align = 8);

    constructor { name: ::std::string::String, size: u32 };
}
```

But it doesn't solve the problem, since we can't create a `String` so we can't call the constructor.
To creating a `String`, we declare the primitive type `str` and its `to_owned` method:

```
type str {
    wellknown_traits(?Sized);

    fn to_owned(&self) -> ::std::string::String;
}
```

There are some new things here.
First, since `str` is a primitive it doesn't need full path.
Then there is `wellknown_traits(?Sized)` instead of `#layout(size = X, align = Y)`
which tells Zngur that this type is unsized and it should consider its references as fat
and prevent storing it by value.

Now you may wonder how we can obtain a `&str` to make a `String` from it?
Fortunately, Zngur has some special support for primitive types
and it has a `operator""_rs` that creates a `&str` from a zero terminated, valid UTF8 `char*` with the static lifetime.
If Zngur didn't have this, we could create a `&[u8]` by exporting its `from_raw_parts` and then converting it to a `&str`,
but Zngur tries to mimic Rust language features in C++ with a similar syntax.

So now we can finally use the `add_item` method:

```C++
int main() {
  auto inventory = rust::crate::Inventory::new_empty(1000);
  inventory.add_banana(3);
  inventory.add_item(rust::crate::Item("apple"_rs.to_owned(), 5));
  zngur_dbg(inventory);
}
```

```
[main.cpp:8] inventory = Inventory {
    items: [
        Item {
            name: "banana",
            size: 7,
        },
        Item {
            name: "banana",
            size: 7,
        },
        Item {
            name: "banana",
            size: 7,
        },
        Item {
            name: "apple",
            size: 5,
        },
    ],
    remaining_space: 974,
}
```

## Generic types

Let's try to add and bridge the `into_items` method:

```Rust
impl Inventory {
    fn into_items(self) -> Vec<Item> {
        self.items
    }
}
```

`Vec` is a generic type, but the syntax to use it is not different:

```
type ::std::vec::Vec<crate::Item> {
    #layout(size = 24, align = 8);
    wellknown_traits(Debug);
}

type crate::Inventory {
    // ...
    fn into_items(self) -> ::std::vec::Vec<crate::Item>;
}
```

Note that this only brings `Vec<Item>`,
for using `Vec<i32>` or `Vec<String>` or `Vec<SomethingElse>` you need to add each of them separately.

Now you can use `into_items` method in C++:

```C++
rust::std::vec::Vec<rust::crate::Item> v = inventory.into_items();
zngur_dbg(v);
```

```
[main.cpp:11] v = [
    Item {
        name: "banana",
        size: 7,
    },
    Item {
        name: "banana",
        size: 7,
    },
    Item {
        name: "banana",
        size: 7,
    },
    Item {
        name: "apple",
        size: 5,
    },
]
```

You can see the full code at [`examples/tutorial`](https://github.com/HKalbasi/zngur/blob/main/examples/tutorial)

## Calling C++ from Rust

C++/Rust interop has two sides, and no interop tool is complete without supporting both.
Here, we will do the reverse of the above task, swapping the Rust and C++ rules.
So, let's assume we have this C++ code:

```C++
#include <string>
#include <vector>

namespace cpp_inventory {
struct Item {
  std::string name;
  uint32_t size;
};

struct Inventory {
  std::vector<Item> items;
  uint32_t remaining_space;

  Inventory(uint32_t space) : items(), remaining_space(space) {}

  void add_item(Item item) {
    remaining_space -= item.size;
    items.push_back(std::move(item));
  }

  void add_banana(uint32_t count) {
    add_item(Item{
        .name = "banana",
        .size = 7,
    });
  }
};

} // namespace cpp_inventory
```

Create a new cargo project, this time a binary one since we want to write the main function to live inside Rust.
Copy the above code into the `inventory.h` file at the root of the crate.
In the same directory, create a `main.zng` file with the following content:

```
#cpp_additional_includes "
    #include <inventory.h>
"
// "file" includes can be written using rust raw string syntax:
// #cpp_additional_includes r#"
//     #include "inventory.h"
// "#

type c++::Inventory {
    #layout(size = 16, align = 8);

    #cpp_heap_allocated "::cpp_inventory::Inventory";
}

type c++::Item {
    #layout(size = 16, align = 8);

    #cpp_heap_allocated "::cpp_inventory::Item";
}
```

And add these to the `main.rs` file:

```Rust
mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}
```

This time we will use the Zngur generator inside of cargo build script.
We could still use the `zngur-cli` but in projects where cargo is the boss, using build script is better.
Add `zngur` and `cc` to your build dependencies:

```toml
[build-dependencies]
cc = "1.0"
build-rs = "0.3.4" # This one is optional
zngur = "0.10.0" # Or whatever the latest version is
```

Then fill the `build.rs` file:

```Rust
use zngur::Zngur;

fn main() {
    build_rs::output::rerun_if_changed("main.zng");
    build_rs::output::rerun_if_changed("impls.cpp");
    build_rs::output::rerun_if_env_changed("CXX");

    let crate_dir = build_rs::input::cargo_manifest_dir();
    let out_dir = build_rs::input::out_dir();

    Zngur::from_zng_file(crate_dir.join("main.zng"))
        .with_cpp_file(out_dir.join("generated.cpp"))
        .with_h_file(out_dir.join("generated.h"))
        .with_rs_file(out_dir.join("generated.rs"))
        .with_crate_name("crate")
        .with_zng_header_in_place()
        .generate();

    let my_build = &mut cc::Build::new();
    let my_build = my_build
	    .cpp(true)
		.std("c++20");
        .include(&crate_dir)
        .include(&out_dir);
    let my_build = || my_build.clone();

    my_build()
        .file(out_dir.join("generated.cpp"))
        .compile("zngur_generated");
    my_build().file("impls.cpp").compile("impls");
}
```

Now we have a `c++::Inventory` and a `c++::Item` that can contain their C++ counterparts.
But there is no way to use them in Rust.
In Zngur, the Rust side can't access C++ opaque objects.
So to make these types useful in Rust, we can add `impl` blocks for these types in C++.
Add this to the `main.zng`:

```
type str {
    wellknown_traits(?Sized);

    fn as_ptr(&self) -> *const u8;
    fn len(&self) -> usize;
}

extern "C++" {
    impl c++::Inventory {
        safe fn new_empty(u32) -> c++::Inventory;
        safe fn add_banana(&mut self, u32);
        safe fn add_item(&mut self, c++::Item);
    }

    impl c++::Item {
        safe fn new(&str, u32) -> c++::Item;
    }
}
```

Now we can define these methods in the C++ and use them in Rust.
Create a file named `impls.cpp` with this content:

```C++
#include "generated.h"
#include <string>

namespace rust {

Inventory Impl<Inventory>::new_empty(uint32_t space) {
  return Inventory::build(space);
}

Unit Impl<Inventory>::add_banana(RefMut<Inventory> self, uint32_t count) {
  self.cpp().add_banana(count);
  return {};
}

Unit Impl<Inventory>::add_item(RefMut<Inventory> self, Item item) {
  self.cpp().add_item(item.cpp());
  return {};
}

Item Impl<Item>::new_(Ref<Str> name, uint32_t size) {
  return Item::build(cpp_inventory::Item{
      .name = ::std::string(reinterpret_cast<const char *>(name.as_ptr()),
                            name.len()),
      .size = size});
}

} // namespace rust
```

These functions look like some unnecessary boilerplate, but writing them has some benefits:

- We can convert C++ types to the Rust equivalents in these functions.
  For example, converting a pointer and length to a slice,
  or `&str` to `std::string` that happened in the `Item::new` above.
- We can convert exceptions to Rust `Result` or `Option`.
- We can control the signature of methods, and use proper lifetimes and mutability for references.
  In case of mutability, Rust mutability means exclusiveness,
  which might be too restrictive and we may want to consider the C++ type interior mutable.
  We can also add nullability with `Option` or make the function `unsafe`.
- We can choose Rusty names for the functions (like `new` and `len`)
  or put the functionality in the proper trait
  (for example implementing the `Iterator` trait instead of exposing the `.begin` and `.end` functions)

Even in the tools that support calling C++ functions directly,
people often end up writing Rust wrappers around C++ types for these reasons.
In Zngur, that code is the wrapper, which lives in the C++ so it can do whatever C++ does.

In the Rust to C++ side, we used `zngur_dbg` macro to see the result.
We will do the same here with the `dbg!` macro.
To do that, we need to implement the `Debug` trait for `c++::Inventory`.
Add this to the `main.zng`:

```
// ...
type ::std::ffi::CStr {
    wellknown_traits(?Sized);

    fn from_ptr(*const i8) -> &::std::ffi::CStr;
    fn to_str(&self) -> ::std::result::Result<&str, ::std::str::Utf8Error>;
}

type ::std::result::Result<&str, ::std::str::Utf8Error> {
    #layout(size = 24, align = 8);

    fn expect(self, &str) -> &str;
}

type ::std::fmt::Result {
    #layout(size = 1, align = 1);
}

type ::std::fmt::Formatter {
    #only_by_ref;

    fn write_str(&mut self, &str) -> ::std::fmt::Result;
}

extern "C++" {
    // ...

    impl std::fmt::Debug for c++::Inventory {
        safe fn fmt(&self, &mut ::std::fmt::Formatter) -> ::std::fmt::Result;
    }
}
```

and this code to the `impls.cpp`:

```C++
namespace rust {

using std::ffi::CStr;
using std::fmt::Debug;
using std::fmt::Formatter;
using std::fmt::Result;

static Ref<Str> rust_str_from_c_str(const char* input) {
  return CStr::from_ptr(reinterpret_cast<const int8_t*>(input)).to_str().expect("invalid_utf8"_rs);
}

Result Impl<Inventory, Debug>::fmt(Ref<Inventory> self, RefMut<Formatter> f) {
  ::std::string result = "Inventory { remaining_space: ";
  result += ::std::to_string(self.cpp().remaining_space);
  result += ", items: [";
  bool is_first = true;
  for (const auto &item : self.cpp().items) {
    if (!is_first) {
      result += ", ";
    } else {
      is_first = false;
    }
    result += "Item { name: \"";
    result += item.name;
    result += "\", size: ";
    result += ::std::to_string(item.size);
    result += " }";
  }
  result += "] }";
  return f.write_str(rust_str_from_c_str(result.c_str()));
}

} // namespace rust
```

So now we can write the main function:

```Rust
use generated::Inventory;
use generated::Item;

fn main() {
    let mut inventory = Inventory::new_empty(1000);
    inventory.add_banana(3);
    inventory.add_item(Item::new("apple", 5));
    dbg!(inventory);
}
```

and run it:

```
[examples/tutorial_cpp/src/main.rs:12] inventory = Inventory { remaining_space: 974, items: [Item { name: "banana", size: 7 }, Item { name: "banana", size: 7 }, Item { name: "banana", size: 7 }, Item { name: "apple", size: 5 }] }
```

You can see the full code at [`examples/tutorial_cpp`](https://github.com/HKalbasi/zngur/blob/main/examples/tutorial_cpp)
