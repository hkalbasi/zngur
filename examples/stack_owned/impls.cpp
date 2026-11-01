#include "cpp_type.h"

#include <iostream>

#include "generated.h"

namespace rust {

using cpp::MyCppWrapper;

MyCppWrapper Impl<MyCppWrapper>::new_(int32_t x, int32_t y) {
  return MyCppWrapper::build(x, y);
}

Unit Impl<MyCppWrapper>::print(Ref<MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  std::cout << "CppType " << cpp.x << " " << cpp.y << std::endl;
  return {};
}

} // namespace rust
