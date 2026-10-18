#include <cstdint>
#include <iostream>

#include "generated.h"

int main() {
  rust::Tuple<int32_t, rust::std::string::String> t{42, "hi"_rs.to_owned()};

  // Read value
  int32_t v = t.f0; // operator T() on FieldOwned<int32_t, 0>
  std::cout << "v = " << v << "\n";

  // Get a Ref<T> from a field
  rust::Ref<int32_t> r = t.f0;
  std::cout << "*r = " << *r << "\n";

  // Access methods through Ref from Field wrappers
  rust::Ref<rust::std::string::String> sref = t.f1;
  std::cout << "sref.len() = " << sref.len() << "\n";

  // From references to container, fields become FieldRef/FieldRefMut
  rust::Ref<decltype(t)> rt = t;
  std::cout << "rt.f1.len() = " << rt.f1.len() << "\n";

  rust::RefMut<decltype(t)> mt = t;
  mt.f1.push_str("!"_rs);
  std::cout << "mt.f1.len() = " << mt.f1.len() << "\n";
}
