#include "cpp_type.h"

#include <iostream>

#include "generated.h"

namespace rust {

rust::MyCppWrapper Impl<rust::MyCppWrapper>::new_(int32_t x, int32_t y) {
  return rust::MyCppWrapper::build(x, y);
}

rust::Unit
Impl<rust::MyCppWrapper>::print(rust::Ref<rust::MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  std::cout << "CppType " << cpp.x << " " << cpp.y << std::endl;
  return {};
}

} // namespace rust
