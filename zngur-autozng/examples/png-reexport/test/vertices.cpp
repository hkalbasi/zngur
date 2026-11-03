// End-to-end check of the generated C++ interface for the `png-reexport` example.
//
// This file is compiled by `tests/autozng/png_reexport/main.nu` against the header generated
// from the example's `.zng`, and linked against the example's Rust staticlib. It must be run
// with `quadrants.png` in the working directory.
//
// A vertex is a pixel corner `(x, y)` whose 2x2 neighborhood contains at least three distinct
// colors. For the test image those are the six places where the quadrant borders and the
// centred square's edges meet; the printed color order is
// `(x-1, y-1), (x, y-1), (x-1, y), (x, y)`.

#include "generated.h"

#include <cstdint>
#include <cstdio>
#include <set>
#include <vector>

static const char* color_name(std::uint32_t rgb) {
    switch (rgb) {
    case 0x0000ff:
        return "blue";
    case 0xffff00:
        return "yellow";
    case 0x00ff00:
        return "green";
    case 0xff0000:
        return "red";
    case 0xffffff:
        return "white";
    case 0x000000:
        return "black";
    default:
        return "unknown";
    }
}

int main() {
    auto reader = rust::crate::open("quadrants.png"_rs);

    auto info = reader.info();
    std::uint32_t width = std::uint32_t(info.width);
    std::uint32_t height = std::uint32_t(info.height);
    if (width != 256 || height != 256) {
        std::printf("unexpected size %ux%u\n", width, height);
        return 1;
    }

    auto color_type = rust::Ref(info.color_type);
    if (!rust::crate::ColorType::Rgb::match_ref(color_type)) {
        std::puts("unexpected color type");
        return 2;
    }

    auto bit_depth = rust::Ref(info.bit_depth);
    if (!rust::crate::BitDepth::Eight::match_ref(bit_depth)) {
        std::puts("unexpected bit depth");
        return 3;
    }

    // `next_frame` takes `&mut [u8]`; build a mutable slice view over the pixel buffer with the
    // raw-parts constructor. The `.zng` slice methods (`len`, `as_ptr`) cover the opposite
    // direction: decoding paths that return a slice to C++.
    std::size_t buffer_size = reader.output_buffer_size().unwrap();
    std::vector<std::uint8_t> pixels(buffer_size);
    auto buffer = rust::RawMut<rust::Slice<std::uint8_t>>::from_raw_parts(
                      reinterpret_cast<std::uintptr_t>(pixels.data()), pixels.size())
                      .as_mut_unchecked();
    auto frame = reader.next_frame(buffer).unwrap();
    std::size_t frame_size = frame.buffer_size();
    if (frame_size > pixels.size()) {
        std::printf("frame size %zu exceeds buffer %zu\n", frame_size, pixels.size());
        return 4;
    }

    auto pixel = [&](std::uint32_t x, std::uint32_t y) {
        std::size_t index = (std::size_t(y) * width + x) * 3;
        return (std::uint32_t(pixels[index]) << 16) | (std::uint32_t(pixels[index + 1]) << 8) |
               std::uint32_t(pixels[index + 2]);
    };

    for (std::uint32_t y = 1; y < height; ++y) {
        for (std::uint32_t x = 1; x < width; ++x) {
            std::uint32_t a = pixel(x - 1, y - 1);
            std::uint32_t b = pixel(x, y - 1);
            std::uint32_t c = pixel(x - 1, y);
            std::uint32_t d = pixel(x, y);
            std::set<std::uint32_t> colors{a, b, c, d};
            if (colors.size() >= 3) {
                std::printf(
                    "(%u,%u) %s %s %s %s\n",
                    x,
                    y,
                    color_name(a),
                    color_name(b),
                    color_name(c),
                    color_name(d));
            }
        }
    }
    return 0;
}
