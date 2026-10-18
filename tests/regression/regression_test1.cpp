#include <iostream>
#include <vector>

#include "./generated.h"

void test_dbg_works_for_ref_and_refmut() {
  auto scope =
      rust::crate::Scoped::new_("Test dbg works for Ref and RefMut"_rs);

  rust::Ref<rust::Str> v1 = "foo"_rs;
  zngur_dbg(v1);
  rust::std::string::String v2 = v1.to_owned();
  zngur_dbg(v2);
  rust::Ref<rust::std::string::String> v3 = v2;
  zngur_dbg(v3);
  rust::std::string::String v4 = std::move(zngur_dbg(v2));
  zngur_dbg(v4);
  rust::RefMut<rust::std::string::String> v5 = v4;
  zngur_dbg(v5);
  v5.push_str(zngur_dbg("bar"_rs));
  zngur_dbg(v4);
}

template <typename T>
concept has_push_str = requires(T v, rust::Ref<rust::Str> s) { v.push_str(s); };

void test_fields_and_constructor() {
  auto scope = rust::crate::Scoped::new_("Test fields and constructor work"_rs);

  rust::crate::Foo v1 = rust::crate::Foo{1, "bar"_rs.to_owned()};
  zngur_dbg(v1);
  zngur_dbg(v1.field2);
  zngur_dbg(v1.field2.len());
  v1.field2.push_str("baz"_rs);
  zngur_dbg(v1);

  rust::Tuple<rust::std::string::String, rust::crate::Foo> v2{
      "kkk"_rs.to_owned(), std::move(v1)};
  zngur_dbg(v2);
  zngur_dbg(v2.f0);
  zngur_dbg(v2.f1);
  zngur_dbg(v2.f1.field2);
  v2.f1.field2.push_str("xxx"_rs);

  rust::Ref<rust::Tuple<rust::std::string::String, rust::crate::Foo>> v3 = v2;
  zngur_dbg(v3.f0);
  zngur_dbg(v3.f1);
  zngur_dbg(v3.f1.field2);
  static_assert(has_push_str<decltype(v2.f1.field2)>);
  static_assert(!has_push_str<decltype(v3.f1.field2)>);
  zngur_dbg(v3.f1.field2.len());

  rust::RefMut<rust::Tuple<rust::std::string::String, rust::crate::Foo>> v4 =
      v2;
  zngur_dbg(v4.f0);
  zngur_dbg(v4.f1);
  zngur_dbg(v4.f1.field2);
  v4.f1.field2.push_str("yyy"_rs);
  zngur_dbg(v4.f1.field2.len());
}

void test_field_underlying_conversions() {
  auto scope =
      rust::crate::Scoped::new_("Test Field* underlying conversions"_rs);

  rust::Tuple<int32_t, rust::std::string::String> pair{42, "hi"_rs.to_owned()};

  // FieldOwned conversion to Ref and value
  rust::Ref<int32_t> r0 = pair.f0;
  int32_t v0 = pair.f0;
  zngur_dbg(v0);
  // Types which are not `Copy` cannot support implicit conversion to T.
  // We must use `.clone()` or similar methods to get a copy.
  rust::std::string::String v1 = pair.f1.clone();
  zngur_dbg(v1);

  // FieldOwned<String> to Ref<String> and call a method
  rust::Ref<rust::std::string::String> sref = pair.f1;
  zngur_dbg(sref.len());

  rust::Ref<rust::Tuple<int32_t, rust::std::string::String>> pref = pair;
  zngur_dbg(int32_t(pref.f0));
  zngur_dbg(pref.f1.len());

  rust::RefMut<rust::Tuple<int32_t, rust::std::string::String>> pmut = pair;
  zngur_dbg(int32_t(pmut.f0));
  pmut.f1.push_str("!"_rs);
  zngur_dbg(pmut.f1.len());
}

void test_floats() {
  auto scope = rust::crate::Scoped::new_("Test floats"_rs);

  rust::Tuple<float, double> pair{42.24, 12.3};

  // FieldOwned conversion to Ref and value
  rust::Ref<double> r1 = pair.f1;
  zngur_dbg(*r1);
  double v1 = pair.f1;
  zngur_dbg(v1);

  rust::std::vec::Vec<float> fvec = rust::std::vec::Vec<float>::new_();
  fvec.push(pair.f0);
  fvec.push(147);
  zngur_dbg(fvec);
  zngur_dbg(fvec.get(0));
  zngur_dbg(fvec.get(2));
  zngur_dbg(*fvec.get(1).unwrap());
  *fvec.get_mut(1).unwrap() = 5.43;
  zngur_dbg(fvec);
}

void test_dyn_fn_with_multiple_arguments() {
  auto scope = rust::crate::Scoped::new_("Test dyn Fn() with multiple arguments"_rs);
  rust::crate::call_dyn_fn_multi_args(rust::Box<rust::Dyn<rust::Fn<int32_t, rust::crate::Scoped, rust::Ref<rust::Str>, rust::Unit>>>::make_box(
      [](int32_t arg0, rust::crate::Scoped arg1, rust::Ref<rust::Str> arg2) {
        std::cout << "Inner function called" << std::endl;
        return rust::Unit{};
      }
  ));
}

void test_refref() {
  auto scope = rust::crate::Scoped::new_("Test Ref<Ref<T>>"_rs);

  rust::std::vec::Vec<rust::Ref<rust::Str>> strvec = rust::std::vec::Vec<rust::Ref<rust::Str>>::new_();

  strvec.push("a str"_rs);
  strvec.push("foobar"_rs);
  strvec.push("a third str"_rs);
  zngur_dbg(strvec);
  zngur_dbg(strvec.get(0));
  zngur_dbg(strvec.get(2));
  // Verify we can get the top level address of a double reference type
  // NOTE: We don't print it since it isn't stable for a diff test
  rust::Raw<rust::Ref<rust::Str>>(strvec.get(0).unwrap()).addr();
  rust::Raw<rust::Ref<rust::Str>>(strvec.get(2).unwrap()).addr();
  zngur_dbg(*strvec.get(1).unwrap());
  *strvec.get_mut(1).unwrap() = "flip flop"_rs;
  zngur_dbg(strvec);
}

void test_zero_sized_type() {
  auto scope = rust::crate::Scoped::new_("Test zero-sized type"_rs);
  auto zst = rust::crate::ZeroSizedType::new_();
  zst.method();
  zngur_dbg(zst);
}

void test_nested_heap_refs_and_auto_field_offset() {
  auto scope = rust::crate::Scoped::new_("Test nested Ref<T> where T is #heap_allocated and auto field offsets"_rs);

  auto a = ::rust::crate::TypeA { 
    10,
    ::rust::crate::FieldTypeA {
      ::rust::crate::FieldTypeC { 20, 30, 40 }
    },
    ::rust::crate::FieldTypeB {
      ::rust::crate::FieldTypeC { 50, 60, 70 }
    },
  };
  zngur_dbg(a);
  zngur_dbg(::rust::Ref<int32_t>(a.foo));
  zngur_dbg(::rust::Ref<int32_t>(a.bar.fizz.buzz_2));
  zngur_dbg(::rust::RefMut<int32_t>(a.baz.fizz.buzz_3));

  auto a_fa_fizz = ::rust::Ref<::rust::crate::FieldTypeC>(a.bar.fizz);
  zngur_dbg(::rust::Ref<int32_t>(a_fa_fizz.buzz_1));
  auto a_fb_fizz = ::rust::Ref<::rust::crate::FieldTypeC>(a.baz.fizz);
  zngur_dbg(::rust::Ref<int32_t>(a_fb_fizz.buzz_2));

  auto b = ::rust::crate::TypeB { 
    100,
    ::rust::crate::FieldTypeA {
      ::rust::crate::FieldTypeC { 200, 300, 400 }
    },
    ::rust::crate::FieldTypeB {
      ::rust::crate::FieldTypeC { 500, 600, 700 }
    },
  };
  zngur_dbg(b);
  zngur_dbg(::rust::Ref<int32_t>(b.foo));
  zngur_dbg(::rust::Ref<int32_t>(b.bar.fizz.buzz_2));
  zngur_dbg(::rust::RefMut<int32_t>(b.baz.fizz.buzz_3));

  auto b_fa_fizz = ::rust::Ref<::rust::crate::FieldTypeC>(b.bar.fizz);
  zngur_dbg(::rust::Ref<int32_t>(b_fa_fizz.buzz_1));
  auto b_fb_fizz = ::rust::Ref<::rust::crate::FieldTypeC>(b.baz.fizz);
  zngur_dbg(::rust::Ref<int32_t>(b_fb_fizz.buzz_2));

  auto fa = ::rust::crate::FieldTypeA {
      ::rust::crate::FieldTypeC { 21, 31, 41 }
  };
  zngur_dbg(fa);
  zngur_dbg(::rust::Ref<int32_t>(fa.fizz.buzz_1));
  zngur_dbg(::rust::Ref<int32_t>(fa.fizz.buzz_3));

  auto fb = ::rust::crate::FieldTypeB {
      ::rust::crate::FieldTypeC { 51, 61, 71 }
  };
  zngur_dbg(fa);
  zngur_dbg(::rust::Ref<int32_t>(fb.fizz.buzz_2));
  zngur_dbg(::rust::Ref<int32_t>(fb.fizz.buzz_3));
}

void test_conservative_layout() {
  auto scope = rust::crate::Scoped::new_("Test #layout_conservative"_rs);

  auto c_layout = ::rust::crate::ConservativeLayoutType {
      3.14159, 42, "A string at some unknown offset"_rs.to_owned() 
  };
  zngur_dbg(c_layout);

  std::cout << "Rust( size = " << c_layout.mem_size() << " , align = " << c_layout.mem_align() << " )\n";
  std::cout << "c++( size = " << sizeof(c_layout.__zngur_data) << " , align = " << alignof(decltype(c_layout)) << " )\n";

  rust::std::vec::Vec<::rust::crate::ConservativeLayoutType> layouts = rust::std::vec::Vec<::rust::crate::ConservativeLayoutType>::new_();
  layouts.push(std::move(c_layout));
  layouts.push(
    ::rust::crate::ConservativeLayoutType {
        2.71828, 1000, "Another test string"_rs.to_owned() 
    }
  );
  zngur_dbg(layouts);
  zngur_dbg(layouts.get(0));
  zngur_dbg(layouts.get(1));
  zngur_dbg(layouts.get(1).unwrap());
  *::rust::RefMut<int32_t>(layouts.get_mut(1).unwrap().field2) = 10;
  zngur_dbg(layouts);

}

void test_nested_ref_dbg() {
  auto scope = rust::crate::Scoped::new_("Test dbg of Ref<Ref<T>>"_rs);

  auto foo = rust::crate::SimpleFoo(42);
  auto bar = rust::crate::RefHolder(rust::Ref<rust::crate::SimpleFoo>(foo));
  rust::Ref<rust::crate::RefHolder> r = bar;
  rust::Ref<rust::Ref<rust::crate::RefHolder>> rr = r;
  rust::Ref<rust::Ref<rust::Ref<rust::crate::RefHolder>>> rrr = rr;
  zngur_dbg(r);
  zngur_dbg(rr);
  zngur_dbg(rrr);
  zngur_dbg(rust::Ref<rust::Ref<rust::crate::SimpleFoo>>(r.f0));
  zngur_dbg(rust::Ref<rust::Ref<rust::crate::SimpleFoo>>((*rr).f0));
  zngur_dbg(rust::Ref<rust::Ref<rust::crate::SimpleFoo>>((**rrr).f0));
}

extern "C" void regression_test1_main() {
  test_dbg_works_for_ref_and_refmut();
  test_fields_and_constructor();
  test_field_underlying_conversions();
  test_floats();
  test_dyn_fn_with_multiple_arguments();
  test_refref();
  test_zero_sized_type();
  test_nested_heap_refs_and_auto_field_offset();
  test_conservative_layout();
  test_nested_ref_dbg();
}
