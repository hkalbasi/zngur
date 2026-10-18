#pragma once
#include <cstdint>

namespace task {

enum class Poll {
  kPending,
  kReady,
};

class Task {
public:
  virtual ~Task() {}
  virtual Poll poll() = 0;
};

class Dispatcher {
public:
  void run_task(Task* task) {
    while (task->poll() == Poll::kPending) {
    }
  }
};

} // namespace task

