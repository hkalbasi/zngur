#include <cstdint>
#include <iostream>

#include "generated.h"

extern "C" void enum_variant_field_access_main() {
  rust::crate::Choice first = rust::crate::Choice::first();
  if (auto variant = rust::crate::Choice::First::match_ref(first)) {
    std::cout << "first.value = " << int32_t(variant->value) << "\n";
  }

  rust::crate::Choice second = rust::crate::Choice::second();
  if (auto variant = rust::crate::Choice::Second::match_mut(second)) {
    std::cout << "second.value = " << int32_t(variant->value) << "\n";
    *rust::RefMut<int32_t>(variant->value) = 7;
    std::cout << "second.value = " << int32_t(variant->value) << "\n";
  }

  rust::Ref<rust::crate::Choice> first_ref(first);
  if (auto variant = rust::crate::Choice::First::match_ref(first_ref)) {
    std::cout << "first_ref.value = " << int32_t(variant->value) << "\n";
  }

  rust::RefMut<rust::crate::Choice> second_ref(second);
  if (auto variant = rust::crate::Choice::Second::match_mut(second_ref)) {
    std::cout << "second_ref.value = " << int32_t(variant->value) << "\n";
    *rust::RefMut<int32_t>(variant->value) = 9;
    std::cout << "second_ref.value = " << int32_t(variant->value) << "\n";
  }
}
