#include "generated.h"

#include <utility>

namespace rust::exported_functions {
int32_t verify_send_sync_bounds() {
  auto boxed_callback =
      rust::Box<rust::Dyn<rust::Fn<int32_t, int32_t>, rust::Send, rust::Sync>>::make_box(
          [](int32_t value) { return value + 2; });
  if (rust::crate::call_boxed(::std::move(boxed_callback), 40) != 42) {
    return 0;
  }

  auto arc_callback = rust::crate::make_arc();
  return rust::crate::call_arc(::std::move(arc_callback), 39) == 42 ? 1 : 0;
}
}  // namespace rust::exported_functions
