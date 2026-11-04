#include "generated.h"

// A raw pointer to a reference renders as `Raw<Ref<T>>` / `RawMut<RefMut<T>>`.
// Calling these methods instantiates `__zngur_internal_data_ptr` for those types;
// GCC rejects the generated call unless the `__zngur_internal` specialization defines
// `data_ptr`.
void use_raw_pointer_to_reference(rust::crate::Thing &thing) {
  auto immutable = thing.as_ptr();
  auto mutable_ptr = thing.as_mut_ptr();
  (void)immutable;
  (void)mutable_ptr;
}
