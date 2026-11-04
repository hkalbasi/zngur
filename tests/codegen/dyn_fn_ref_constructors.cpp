#include "generated.h"

#include <functional>
#include <type_traits>
#include <utility>

using FnDyn = rust::Dyn<rust::Fn<int32_t, rust::Bool>>;
using Fn = std::function<rust::Bool(int32_t)>;
using FnRef = rust::Ref<FnDyn>;
using FnRefMut = rust::RefMut<FnDyn>;

// `Ref*<Dyn<Fn*<...>>>` constructors must be named after the wrapper type and
// take the `std::function` signature of the `Fn` trait.
static_assert(std::is_constructible_v<FnRef, Fn>);
static_assert(std::is_constructible_v<FnRefMut, Fn>);
static_assert(std::is_constructible_v<FnRef, FnRefMut>);
