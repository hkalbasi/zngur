//! Test crate whose public API is a glob re-export of the external `png` crate, plus a small
//! `Image` API used by the end-to-end C++ test.
//!
//! The glob exercises autozng's handling of re-exported *external* types: every re-exported item
//! the generated `.zng` names lives in `png`, not in this crate, so it is only reachable through
//! the re-export pass (`analyze::visit_reexported_items`).
//!
//! `Image` is the interface `test/vertices.cpp` drives through the generated C++ bridge: it loads
//! the test image with the re-exported `png` decoder and exposes just enough to inspect pixels.

#![cfg_attr(autozng, feature(register_tool, custom_inner_attributes))]
#![cfg_attr(autozng, register_tool(autozng))]

pub use png::*;

/// A decoded PNG image.
pub struct Image {
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    pixels: Vec<u8>,
}

impl Image {
    /// Decode `path` with the re-exported `png` decoder.
    pub fn load(path: &str) -> Image {
        let file = std::fs::File::open(path).expect("opening the test image");
        let decoder = Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder.read_info().expect("reading the PNG header");
        let mut pixels = vec![0; reader.output_buffer_size().expect("buffer size")];
        let info = reader.next_frame(&mut pixels).expect("decoding the PNG");
        pixels.truncate(info.buffer_size());
        Image {
            width: info.width,
            height: info.height,
            color_type: info.color_type,
            bit_depth: info.bit_depth,
            pixels,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn color_type(&self) -> ColorType {
        self.color_type
    }

    pub fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    /// The RGB value at `(x, y)` as `0xRRGGBB`.
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        assert!(x < self.width && y < self.height);
        let i = ((y * self.width + x) * 3) as usize;
        u32::from(self.pixels[i]) << 16
            | u32::from(self.pixels[i + 1]) << 8
            | u32::from(self.pixels[i + 2])
    }
}
