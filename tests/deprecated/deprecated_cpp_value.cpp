#include "deprecated_cpp_value.h"

#include "generated.h"

using Accumulator = ::rust::crate::Accumulator;

Accumulator rust::Impl<Accumulator>::new_() {
  return Accumulator(::rust::ZngurCppOpaqueOwnedObject::build<::cpp_accumulator::Accumulator>());
}

rust::Unit rust::Impl<Accumulator>::add(rust::RefMut<Accumulator> self, uint32_t x) {
  self.cpp().add(x);
  return {};
}

uint32_t rust::Impl<Accumulator>::total(rust::Ref<Accumulator> self) {
  return self.cpp().total();
}
