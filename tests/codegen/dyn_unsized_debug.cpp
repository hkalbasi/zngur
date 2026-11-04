#include "generated.h"

#include <type_traits>
#include <utility>

using DebugAny = rust::crate::DebugAny;
using DebugAnySend = rust::Dyn<DebugAny, rust::Send>;
using DebugAnySendRef = rust::Ref<DebugAnySend>;
using DebugAnySendRefMut = rust::RefMut<DebugAnySend>;

// A `dyn DebugAny + Send` with the `Debug` well-known trait must be represented as one
// `rust::Dyn`, and its debug printer must be reachable through `Ref`/`RefMut` of
// that exact type.
static_assert(rust::zngur_is_unsized<DebugAnySend>::value);
static_assert(std::is_same_v<decltype(&rust::ZngurPrettyPrinter<DebugAnySendRef>::print),
                             void (*)(DebugAnySendRef const&)>);
static_assert(std::is_same_v<decltype(&rust::ZngurPrettyPrinter<DebugAnySendRefMut>::print),
                             void (*)(DebugAnySendRefMut const&)>);

namespace rust::exported_functions {
rust::Unit debug_dyn(DebugAnySendRef value) {
    rust::ZngurPrettyPrinter<DebugAnySendRef>::print(value);
    return {};
}
} // namespace rust::exported_functions
