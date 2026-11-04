#include "generated.h"

#include <type_traits>
#include <utility>

using DynError = rust::Ref<rust::Dyn<rust::std::error::Error, rust::Send, rust::Sync>>;

// `Holder::error(&self)` must return a reference to one `dyn Error + Send + Sync`; the generated
// C++ represents that as a single `rust::Dyn`, with the reference outside it.
static_assert(std::is_same_v<decltype(std::declval<rust::crate::Holder&>().error()), DynError>);
static_assert(std::is_same_v<
              decltype(rust::crate::Holder::error(std::declval<rust::Ref<rust::crate::Holder>>())),
              DynError>);

// Instantiate the interface so the compiler checks the call, not just the declarations.
void use_dyn_error_interface(rust::crate::Holder& holder) {
    DynError error = holder.error();
    (void)error;
}
