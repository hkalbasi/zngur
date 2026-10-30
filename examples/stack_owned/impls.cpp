#include "cpp_type.h"

#include <iostream>

#include "generated.h"

namespace rust {

rust::crate::MyCppWrapper Impl<rust::crate::MyCppWrapper>::new_(int32_t x,
                                                                int32_t y) {
  return rust::crate::MyCppWrapper::build(x, y);
}

rust::Unit
Impl<rust::crate::MyCppWrapper>::print(rust::Ref<rust::crate::MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  std::cout << "CppType " << cpp.x << " " << cpp.y << std::endl;
  return {};
}

rust::crate::MyConservativeWrapper
Impl<rust::crate::MyConservativeWrapper>::new_(int64_t a, int32_t b) {
  return rust::crate::MyConservativeWrapper::build(a, b);
}

rust::Unit
Impl<rust::crate::MyConservativeWrapper>::print(
    rust::Ref<rust::crate::MyConservativeWrapper> c) {
  const CppConservativeType &cpp = c.cpp();
  std::cout << "CppConservativeType " << cpp.a << " " << cpp.b << std::endl;
  return {};
}

} // namespace rust
