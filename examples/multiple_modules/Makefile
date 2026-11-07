EXAMPLES_CARGO_PROFILE ?= debug
ZNGUR_CLI = cd ../../zngur-cli && cargo run

a.out: main.cpp aggregation.a packet.zng.h receiver.zng.h aggregation/aggregation.zng.h processor.zng.h zngur.h ../../target/$(EXAMPLES_CARGO_PROFILE)/libexample_multiple_modules.a
	${CXX} -std=c++11 -Werror -I. -Iaggregation -Iprocessor main.cpp aggregation.a -g -L ../../target/$(EXAMPLES_CARGO_PROFILE)/ -l example_multiple_modules

../../target/$(EXAMPLES_CARGO_PROFILE)/libexample_multiple_modules.a:
	cargo build --profile $(EXAMPLES_CARGO_PROFILE:debug=dev)

aggregation.a: aggregation/impls.cpp aggregation/generated.cpp aggregation/aggregation.zng.h zngur.h
	${CXX} -c -std=c++11 -Werror -I. -Iaggregation -o aggregation/generated.o aggregation/generated.cpp
	${CXX} -c -std=c++11 -Werror -I. -Iaggregation -o aggregation/impls.o aggregation/impls.cpp
	ld -r -o aggregation.a aggregation/impls.o aggregation/generated.o
	

packet.zng.h packet/src/packet.zng.rs: packet/packet.zng
	${ZNGUR_CLI} g ../examples/multiple_modules/packet/packet.zng \
		--h-file=../examples/multiple_modules/packet.zng.h \
		--rs-file=../examples/multiple_modules/packet/src/packet.zng.rs \
		--crate-name "packet"

receiver.zng.h receiver/src/receiver.zng.rs: receiver/receiver.zng packet.zng.h
	${ZNGUR_CLI} g ../examples/multiple_modules/receiver/receiver.zng \
		--h-file=../examples/multiple_modules/receiver.zng.h \
		--rs-file=../examples/multiple_modules/receiver/src/receiver.zng.rs \
		--crate-name "receiver"

aggregation/aggregation.zng.h aggregation/src/aggregation.zng.rs aggregation/generated.cpp: aggregation/aggregation.zng packet.zng.h
	${ZNGUR_CLI} g ../examples/multiple_modules/aggregation/aggregation.zng \
		--h-file=../examples/multiple_modules/aggregation/aggregation.zng.h \
		--rs-file=../examples/multiple_modules/aggregation/src/aggregation.zng.rs \
		--crate-name "aggregation"

processor.zng.h processor/src/processor.zng.rs: processor/processor.zng receiver.zng.h aggregation/aggregation.zng.h
	${ZNGUR_CLI} g ../examples/multiple_modules/processor/processor.zng \
		--h-file=../examples/multiple_modules/processor.zng.h \
		--rs-file=../examples/multiple_modules/processor/src/processor.zng.rs \
		--crate-name "processor"

zngur.h:
	${ZNGUR_CLI} -- make-zng-header ../examples/multiple_modules/zngur.h

.PHONY: ../../target/$(EXAMPLES_CARGO_PROFILE)/libexample_multiple_modules.a clean run

run: a.out
	./a.out

clean:
	rm -f *.zng.h aggregation/*.zng.h packet/src/*.zng.rs receiver/src/*.zng.rs aggregation/generated.cpp aggregation/src/*.zng.rs aggregation/*.o processor/src/*.zng.rs zngur.h a.out *.o *.a
