#include <cstdint>
#include <iostream>
#include <type_traits>

#include "generated.h"

using RefOnly = rust::crate::RefOnly;
using Ref = rust::Ref<RefOnly>;
using RefMut = rust::RefMut<RefOnly>;

// A `#only_by_ref` type has no storage of its own; only the handles exist.
// These shape checks were moved from a separate compile-only test.
static_assert(std::is_empty_v<RefOnly>);
static_assert(!std::is_default_constructible_v<RefOnly>);
static_assert(sizeof(Ref) == sizeof(void*));
static_assert(sizeof(RefMut) == sizeof(void*));
static_assert(std::is_constructible_v<Ref, RefMut>);

extern "C" void only_by_ref_field_access_main() {
  Ref r = RefOnly::get();
  std::cout << "r.a = " << int32_t(r.a) << "\n";

  RefMut m = RefOnly::get_mut();
  *rust::RefMut<int32_t>(m.a) = 7;
  std::cout << "m.a = " << int32_t(m.a) << "\n";
}
