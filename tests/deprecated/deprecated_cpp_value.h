#pragma once

#include <cstdint>

namespace cpp_accumulator {

class Accumulator {
public:
  void add(uint32_t x) { value += x; }
  uint32_t total() const { return value; }

private:
  uint32_t value = 0;
};

} // namespace cpp_accumulator
