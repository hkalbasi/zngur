#include "generated.h"

namespace rust::exported_functions {
int32_t cpp_safe() { return 42; }
int32_t cpp_unsafe() { return 43; }

int32_t verify_rust_safety() {
  if (rust::crate::rust_safe() != 42) {
    return 0;
  }
  if (rust::crate::rust_unsafe() != 43) {
    return 0;
  }
  return rust::crate::rust_legacy_unsafe() == 44 ? 1 : 0;
}
} // namespace rust::exported_functions
