#include "cpp_type.h"

#include <iostream>

#include "generated.h"

namespace rust {

using cpp::BoxedInt;
using cpp::MyCppWrapper;
using cpp::OpaqueRef;

MyCppWrapper Impl<MyCppWrapper>::new_(int32_t x, int32_t y) {
  return MyCppWrapper::build(x, y);
}

Unit Impl<MyCppWrapper>::print(Ref<MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  std::cout << "CppType " << cpp.x << " " << cpp.y << std::endl;
  return {};
}

BoxedInt Impl<BoxedInt>::new_(int32_t v) {
  return BoxedInt::build(new int(v));
}

int32_t Impl<BoxedInt>::get(Ref<BoxedInt> b) { return *b.cpp(); }

int32_t Impl<OpaqueRef>::read(Ref<OpaqueRef> r) { return r.cpp().value; }

int32_t exported_functions::run_with_opaque_ref() {
  ::OpaqueRef obj{7};
  return crate::read_opaque(obj);
}

} // namespace rust
