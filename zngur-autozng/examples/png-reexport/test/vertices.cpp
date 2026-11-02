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
#include <variant>

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
    auto image = rust::crate::Image::load("quadrants.png"_rs);

    std::uint32_t width = image.width();
    std::uint32_t height = image.height();
    if (width != 256 || height != 256) {
        std::printf("unexpected size %ux%u\n", width, height);
        return 1;
    }

    auto color_type = image.color_type();
    if (!std::holds_alternative<rust::crate::ColorType::Rgb>(color_type.match())) {
        std::puts("unexpected color type");
        return 2;
    }

    auto bit_depth = image.bit_depth();
    if (!std::holds_alternative<rust::crate::BitDepth::Eight>(bit_depth.match())) {
        std::puts("unexpected bit depth");
        return 3;
    }

    for (std::uint32_t y = 1; y < height; ++y) {
        for (std::uint32_t x = 1; x < width; ++x) {
            std::uint32_t a = image.pixel(x - 1, y - 1);
            std::uint32_t b = image.pixel(x, y - 1);
            std::uint32_t c = image.pixel(x - 1, y);
            std::uint32_t d = image.pixel(x, y);
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
