//@ autozng

// An external type with two instantiations of an external trait: both trait methods have the same
// Rust name, and the type-level renames give them distinct C++ names. The external type cannot
// carry the attribute, so a type alias names it.

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    #[autozng::rename(trait_path = "::std::cmp::PartialEq<str>", suffix = "Str")]
    #[autozng::rename(
        trait_path = "::std::cmp::PartialEq<::std::string::String>",
        suffix = "String"
    )]
    type StringAlias = ::std::string::String;
};

#[cfg(autozng)]
#[autozng::emit_items]
const _: () = {
    let _ = <::std::string::String as PartialEq<str>>::eq;
    let _ = <::std::string::String as PartialEq<::std::string::String>>::eq;
};
