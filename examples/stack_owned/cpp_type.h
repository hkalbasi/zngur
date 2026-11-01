#pragma once
#include <iostream>
#include <memory>
#include <type_traits>
#include <zngur.h>

struct CppType {
  int x;
  int y;

  CppType() = default;
  CppType(const CppType &) = default;

  CppType(int x, int y) : x(x), y(y) {
    std::cout << "Constructed CppType " << x << " " << y << std::endl;
  }
};

struct OpaqueRef {
  int value;
};

// The `.zng` file names C++ types by their C++ path, so these aliases give them the names it uses.
using MyCppWrapper = CppType;
using BoxedInt = std::unique_ptr<int>;
