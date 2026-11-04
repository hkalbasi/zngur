//@ autozng

// `std::io::Error` is defined in the private `std::io::error` module and re-exported at
// `std::io`; the `.zng` must use the public path, not the definition path.
pub fn fail() -> std::io::Error {
    std::io::Error::other("boom")
}
