#include "generated.h"

namespace rust::exported_functions {
int32_t verify() {
  return rust::crate::nested::relative() +
         rust::std::convert::identity(22);
}
}
