#include "generated.h"

namespace rust::exported_functions {
int32_t and_() { return 41; }

int32_t verify_keywords() {
  rust::crate::class_ value(42);
  if (static_cast<int32_t>(value.int_) != 42 || value.xor_() != 42) {
    return 0;
  }

  rust::crate::switch_ variant(rust::crate::switch_::class_{});
  return std::holds_alternative<rust::crate::switch_::class_>(variant.match()) &&
                 and_() == 41
             ? 1
             : 0;
}
} // namespace rust::exported_functions
