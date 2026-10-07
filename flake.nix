{
  description = "Zngur development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
  }: let
    systems = nixpkgs.lib.systems.flakeExposed;
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    devShells = forAllSystems (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {inherit system overlays;};

        # Map Nix system architecture to wasi-sdk release naming conventions
        wasiSystem =
          {
            "x86_64-linux" = "x86_64-linux";
            "aarch64-linux" = "arm64-linux";
            "x86_64-darwin" = "x86_64-macos";
            "aarch64-darwin" = "arm64-macos";
          }.${
            system
          } or (throw "wasi-sdk binaries are not available for your system: ${system}");

        wasi-sdk = pkgs.fetchzip {
          url = "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-30/wasi-sdk-30.0-${wasiSystem}.tar.gz";
          hash =
            {
              "x86_64-linux" = "sha256-QXvKCuEO3PKQdjp1R7IecMfabvtwEPd+ANakcVBxrJA=";
              "x86_64-darwin" = "sha256-FZSgeRMJeBvw0CJEMcNVbsSiMmsgVoe2WfZVDQjYsT4=";
              "aarch64-linux" = "sha256-byl3lCMI2RsBI5eNo8ag1vzngJlLOwIACMYX4mdk6kA=";
              "aarch64-darwin" = "sha256-tLif2+PdtqYPk5emMGkSXDxZw8ybeSbll8649jvHXzg=";
            }.${
              system
            } or (throw "wasi-sdk binaries are not available for your system: ${system}");
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rustfmt" "clippy" "rust-analyzer"];
          targets = ["wasm32-wasip1"];
        };
        # Default shell 
        makeDevShell = stdenv:
          pkgs.mkShell.override {inherit stdenv;} {
            packages = with pkgs;
              [
                cspell
                dprint
                emscripten
                wasmtime
                rustToolchain
                llvmPackages_latest.lld
              ];

            shellHook = ''
              export WASI_SDK_PATH="${wasi-sdk}"
              export EMSDK_PATH="${pkgs.emscripten}/share/emscripten"
              export RUST_BACKTRACE="0"
            '';
          };
      in {
        # Native compiler for the host (Apple Clang on Mac, GCC on Linux)
        default = makeDevShell pkgs.stdenv;

        # Explicitly forces Clang and sets CC=clang, CXX=clang++
        clang = makeDevShell pkgs.clangStdenv;

        # Explicitly forces GCC and sets CC=gcc, CXX=g++
        gcc = makeDevShell pkgs.gccStdenv;
      }
    );
  };
}
