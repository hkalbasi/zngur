#include "impls.h"
#include "generated.h"
#include "task.h"
#include "zngur.h"
#include <new>

namespace rust {

Unit Impl<Dispatcher>::constructor(
    RefMut<Dispatcher> dispatcher) {
  new (&dispatcher.cpp()) task::Dispatcher();
  return {};
}

Unit Impl<CppTask>::constructor(RefMut<CppTask> dispatcher) {
  new (&dispatcher.cpp()) task::CppTaskForRust();
  return {};
}

::rust::Unit Impl<rust::Dispatcher>::run_task(
    ::rust::Ref<rust::Dispatcher> self,
    ::rust::RefMut<rust::crate::RustTask> task) {
  auto &d = self.cpp();
  ::rust::RawMut<::rust::crate::RustTask> raw_mut(task);
  ::rust::crate::RustTask *rust_ptr = ::rust::from_rust_ptr(raw_mut);
  task::CppTaskForRust *task_ref = task::CppTaskForRust::Inheritance::get_base(
      ::rust::as_rust_ptr_mut(rust_ptr));
  d.run_task(task_ref);
  return {};
}

} // namespace rust

task::Poll task::CppTaskForRust::poll() {
  ::rust::RawMut<::rust::crate::RustTask> rust_future =
      Inheritance::get_rust(this);
  ::rust::RefMut<::rust::crate::RustTask> ref_mut =
      rust_future.as_mut_unchecked();
  ::rust::core::task::Poll<::rust::Unit> result =
      ::rust::crate::RustTask::poll(ref_mut);
  return result.is_ready() ? task::Poll::kReady : task::Poll::kPending;
}
