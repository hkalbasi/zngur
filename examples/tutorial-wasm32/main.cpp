#include <cstdio>
#include <iostream>
#include <vector>

#include "./generated.h"

// Rust values are available in the `::rust` namespace from their absolute path
// in Rust
template <typename T> using Vec = rust::std::vec::Vec<T>;
template <typename T> using Option = rust::std::option::Option<T>;
template <typename T> using BoxDyn = rust::Box<rust::Dyn<T>>;

// You can implement Rust traits for your classes
template <typename T>
class VectorIterator : public rust::std::iter::Iterator<T> {
  std::vector<T> vec;
  size_t pos;

public:
  VectorIterator(std::vector<T> &&v) : vec(v), pos(0) {}
  ~VectorIterator() {
    std::cout << "vector iterator has been destructed" << std::endl;
  }

  Option<T> next() override {
    if (pos >= vec.size()) {
      return typename Option<T>::None();
    }
    T value = vec[pos++];
    // You can construct Rust enum with fields in C++
    return typename Option<T>::Some(value);
  }
};

int main() {
  std::setvbuf(stdout, nullptr, _IONBF, 0);

  // You can call Rust functions that return things by value, and store that
  // value in your stack.
  auto s = Vec<int32_t>::new_();
  s.push(2);
  Vec<int32_t>::push(s, 5);
  s.push(7);
  Vec<int32_t>::push(s, 3);
  // You can call Rust functions just like normal Rust.
  std::cout << s.clone().into_iter().sum() << std::endl;
  // Access valid indices
  std::cout << "s[2] = " << *s.get(2).unwrap() << std::endl;
  // TODO: Uncomment this line after enabling wasmtime exceptions.
  // std::cout << "s[4] = " << *s.get(4).unwrap() << std::endl;
  std::cout << "Rust panic would happen if we accessed invalid index, but we avoid it" << std::endl;

  int state = 0;
  // You can convert a C++ lambda into a `Box<dyn Fn>` and friends.
  auto f = BoxDyn<rust::Fn<int32_t, int32_t>>::make_box([&](int32_t x) {
    state += x;
    std::cout << "hello " << x << " " << state << "\n";
    return x * 2;
  });
  // And pass it to Rust functions that accept closures.
  auto x = s.into_iter().map(std::move(f)).sum();
  std::cout << x << " " << state << "\n";
  std::vector<int32_t> vec{10, 20, 60};
  // You can convert a C++ type that implements `Trait` to a `Box<dyn Trait>`.
  // `make_box` is similar to the `make_unique`, it takes constructor arguments
  // and construct it inside the `Box` (instead of `unique_ptr`).
  auto vec_as_iter = BoxDyn<rust::std::iter::Iterator<int32_t>>::make_box<
      VectorIterator<int32_t>>(std::move(vec));
  // Then use it like a normal Rust value.
  auto t = vec_as_iter.collect();
  std::cout.flush();
  std::fflush(stdout);
  // Some utilities are also provided. For example, `zngur_dbg` is the
  // equivalent of `dbg!` macro.
  zngur_dbg(t);
  return 0;
}
