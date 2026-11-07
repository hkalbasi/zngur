#include <iostream>

#include <aggregation.zng.h>
#include <packet.zng.h>
#include <processor.zng.h>
#include <receiver.zng.h>

int main() {
  rust::std::option::Option<rust::processor::Processor> processor = rust::std::option::Option<rust::processor::Processor>::Some(
      rust::processor::Processor::new_());
  rust::std::option::Option<rust::receiver::Receiver> receiver = rust::std::option::Option<rust::receiver::Receiver>::Some(
      rust::receiver::Receiver::new_());
  auto stats =
      rust::Impl<rust::cpp::cpp_stats::StatsAccumulator, rust::Inherent>::create();

  std::cout << "Starting LatencyAnalysis simulation..." << std::endl;

  processor.unwrap().run(receiver.unwrap(), stats, 5);

  rust::Impl<rust::cpp::cpp_stats::StatsAccumulator, rust::Inherent>::print_report(
      stats);

  return 0;
}
