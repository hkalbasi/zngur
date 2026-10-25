#include "generated.h"

namespace rust::exported_functions {
uint8_t read_mut_pointer(uint8_t* ptr) { *ptr += 2; return *ptr; }
uint8_t read_const_pointer(uint8_t const* ptr) { return *ptr; }
}
