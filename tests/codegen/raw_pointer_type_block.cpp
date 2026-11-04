#include <cstddef>

#include "generated.h"

namespace rust::exported_functions {
// The return types name `rust::Raw<...>` / `rust::RawMut<...>`, so compiling these functions
// instantiates the templates.
rust::Tuple<rust::Raw<rust::crate::Point>, size_t> make_tuple() {
  return {rust::Raw<rust::crate::Point>(nullptr), 7};
}

rust::Tuple<rust::RawMut<rust::crate::Point>, size_t> make_mut_tuple() {
  return {rust::RawMut<rust::crate::Point>(nullptr), 9};
}
} // namespace rust::exported_functions
