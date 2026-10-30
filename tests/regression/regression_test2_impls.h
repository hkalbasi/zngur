#pragma once

#include "regression_test2_task.h"
#include "regression_test2_cpp_rust_inheritance.h"

namespace rust {
  namespace crate {
    class RustTask;
  }
}

namespace task {
class CppTaskForRust : public Task {
public:
  virtual Poll poll() override;
  virtual ~CppTaskForRust() override {}
  using Inheritance = ::rust::RustInherit<CppTaskForRust, ::rust::crate::RustTask>;
};
}

namespace rust {
template<>
struct is_trivially_relocatable< ::task::Dispatcher> : std::true_type {};
template<>
struct is_trivially_relocatable< ::task::CppTaskForRust> : std::true_type {};
}
