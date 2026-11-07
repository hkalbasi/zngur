#include <aggregation.zng.h>
#include <packet.zng.h>
#include <stats.h>

using rust::cpp::cpp_stats::StatsAccumulator;
using rust::packet::Packet;

// We need to implement the methods declared in aggregation.zng
StatsAccumulator rust::Impl<StatsAccumulator>::create() {
  return StatsAccumulator(
      rust::ZngurCppOpaqueOwnedObject::build<cpp_stats::StatsAccumulator>());
}

rust::Unit
rust::Impl<StatsAccumulator>::add_packet(rust::RefMut<StatsAccumulator> self,
                                         Packet p) {
  self.cpp().add_packet(p);
  return {};
}

rust::Unit
rust::Impl<StatsAccumulator>::print_report(rust::Ref<StatsAccumulator> self) {
  self.cpp().print_report();
  return {};
}
