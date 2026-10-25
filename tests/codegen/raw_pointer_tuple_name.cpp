#include "generated.h"

namespace rust::exported_functions {
rust::Tuple<uint8_t*, size_t, size_t> make_tuple() {
  static uint8_t value = 42;
  return {&value, 17, 29};
}
}
