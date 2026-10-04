EXAMPLES_CARGO_PROFILE ?= debug

#
# WASI build flags for C++ compilation with wasmtime
WASIFLAGS = -std=c++14 \
           --target=wasm32-wasi \
           --sysroot=$(WASI_SDK_PATH)/share/wasi-sysroot \
           -D_WASI_EMULATED_SIGNAL \
           -lwasi-emulated-signal \
           -fno-exceptions

# Use the mise-installed WASI SDK C++ compiler
# Load mise environment if WASI_SDK_PATH is not set
ifeq ($(WASI_SDK_PATH),)
    $(eval $(shell mise env))
endif
CXX = $(WASI_SDK_PATH)/bin/clang++

# Default target
all: a.out

# Run the example with wasmtime (installs all deps automatically)
run: main.wasm
	wasmtime run --allow-precompiled main.wasm

# CI expects a.out; we create it as a bash script that simply execs wasmtime
a.out: main.wasm
	@printf '%s\n' '#!/usr/bin/env bash' 'exec wasmtime run --allow-precompiled "$$(dirname "$$0")/main.wasm" "$$@"' > $@
	@chmod +x $@

main.wasm: main.cpp generated.h generated.cpp \
           ./target/wasm32-wasip1/$(EXAMPLES_CARGO_PROFILE)/libexample_tutorial_wasm32.a
	$(CXX) $(WASIFLAGS) \
	    main.cpp generated.cpp \
	    ./target/wasm32-wasip1/$(EXAMPLES_CARGO_PROFILE)/libexample_tutorial_wasm32.a \
	    -o $@

./target/wasm32-wasip1/$(EXAMPLES_CARGO_PROFILE)/libexample_tutorial_wasm32.a: wasm32-wasip1-target generated.h ./src/generated.rs ./src/lib.rs
	cargo build --target=wasm32-wasip1 --profile $(EXAMPLES_CARGO_PROFILE:debug=dev)

generated.h ./src/generated.rs generated.cpp: main32.zng
	cargo run --profile $(EXAMPLES_CARGO_PROFILE:debug=dev) --manifest-path ../../zngur-cli/Cargo.toml g -i main32.zng

# Ensure wasm32-wasip1 target is installed
wasm32-wasip1-target:
	@rustup target list --installed | grep -q wasm32-wasip1 || { \
		echo "Installing wasm32-wasip1 Rust target..."; \
		rustup target add wasm32-wasip1; \
	}
	@touch wasm32-wasip1-target

clean:
	cargo clean
	rm -f generated.h ./src/generated.rs generated.cpp generated.o
	rm -f main.wasm example_tutorial_wasm32.wasm a.out
	rm -f wasm32-wasip1-target

FORCE: ;
