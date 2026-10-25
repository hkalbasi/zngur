#include <cstdint>
#include <variant>

#include "generated.h"

namespace rust {
namespace exported_functions {
int32_t ____foo_fn_() { return 42; }
int32_t _Fn12cc(int32_t value) { return value + 42; }
int32_t _verify_names_in_cpp() {
  rust::crate::____foo_ named_field(42);
  if (static_cast<int32_t>(named_field.____field_) != 42) {
    return 0;
  }

  rust::crate::_Ab12cc another_named_type;
  (void)another_named_type;

  rust::crate::_Enum12 named_variant(rust::crate::_Enum12::____Variant_(7));
  auto matched_variant = named_variant.match();
  auto* payload =
      std::get_if<rust::crate::_Enum12::____Variant_>(&matched_variant);
  if (!payload || static_cast<int32_t>(payload->_payload12) != 7) {
    return 0;
  }

  rust::crate::_Enum12 other_variant(rust::crate::_Enum12::_V2cc{});
  return std::holds_alternative<rust::crate::_Enum12::_V2cc>(
             other_variant.match())
             ? 1
             : 0;
}
} // namespace exported_functions
} // namespace rust
