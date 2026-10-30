#include "cpp_stack_owned.h"

#include "generated.h"

namespace rust {

using crate::CfgBox;
using crate::MergedCounter;

MergedCounter Impl<MergedCounter>::new_(int32_t id, int32_t value) {
  return MergedCounter::build(id, value);
}

Unit Impl<MergedCounter>::add(RefMut<MergedCounter> self, int32_t delta) {
  self.cpp().value += delta;
  return {};
}

int32_t Impl<MergedCounter>::get(Ref<MergedCounter> self) {
  return self.cpp().value;
}

int32_t Impl<MergedCounter>::into_value(MergedCounter self) {
  return self.cpp().value;
}

CfgBox Impl<CfgBox>::new_(int64_t payload) {
  return CfgBox::build(payload);
}

int64_t Impl<CfgBox>::get(Ref<CfgBox> self) {
  return self.cpp().payload;
}

} // namespace rust
