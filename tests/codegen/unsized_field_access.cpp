#include <cstdint>
#include <iostream>

#include "generated.h"

namespace rust::exported_functions {
rust::Unit unsized_field_access_main(rust::RefMut<rust::crate::Unsized> m) {
  {
    rust::Ref<rust::crate::Unsized> r = m;
    std::cout << "r.value = " << int32_t(r.value) << "\n";
  }

  *rust::RefMut<int32_t>(m.value) = 7;
  std::cout << "m.value = " << int32_t(m.value) << "\n";
  return {};
}
} // namespace rust::exported_functions
