#pragma once
#include <cstddef>
#include <cstdint>
#include <iostream>
#include <type_traits>

struct TrackedCounter {
  int32_t id;
  int32_t value;

  TrackedCounter() = default;
  TrackedCounter(int32_t id, int32_t value) : id(id), value(value) {
    std::cout << "new " << id << ":" << value << std::endl;
  }
  TrackedCounter(TrackedCounter &&other) noexcept : id(other.id), value(other.value) {
    other.id = -other.id;
  }
  ~TrackedCounter() {
    if (id > 0) {
      std::cout << "drop " << id << ":" << value << std::endl;
    }
  }
};

struct alignas(8) FixedBox {
  int64_t payload;
};
