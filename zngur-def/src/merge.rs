use crate::{
    AdditionalIncludes, ConvertPanicToException, CppHeapAllocated, CppRef, CppStackOwned,
    LayoutPolicy, RustType, ZngurConstructor, ZngurExternCppFn, ZngurExternCppImpl, ZngurField,
    ZngurFn, ZngurMethod, ZngurMethodDetails, ZngurSpec, ZngurTrait, ZngurType, ZngurVariant,
};

/// Trait for types with a partial union operation.
///
/// If a type T is Merge, it provides a partial union operation `merge`: T x T -> T.
///
/// Partial unions do not need to be homogeneous. If a type U is Merge<T>,
/// it provides a partial union operation `merge`: T X U -> U.
/// For example, T: usize, U: Set<usize>; the partial union is the result of
/// adding the lhs usize to the rhs Set.
///
/// "Partial" means the result is not necessarily defined for all inputs; the union may fail.
/// This is often because the instances are contradictory (as defined by the type).
///
/// There are no guarantees about the state of the mutable argument, `into`, in the case
/// of a failed merge. `merge` is not required to leave `into` in a valid state, or restore
/// it to its original state.
pub trait Merge<T = Self> {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// # Errors
    ///
    /// If the instances are contradictory, a `MergeFailure` is returned.
    fn merge(self, into: &mut T) -> MergeResult;
}

/// The result of a merge operation.
pub type MergeResult = Result<(), MergeFailure>;

/// An unsuccessful merge operation.
pub enum MergeFailure {
    /// The merge was not successful because of a conflict.
    /// the second member is a tuple of conflict sources:
    ///     (source in self, source in the item merging into)
    Conflict(String, (ConflictSource, ConflictSource)),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConflictSource {
    Layout,
    Constructor(Option<Vec<(String, RustType)>>),
    CppRef,
    CppHeapAllocated,
    CppStackOwned,
    Method(ZngurMethod),
    Field(String),
}

/// Push an item onto a vector if it is not already present, in linear time.
fn push_unique<T: Eq>(item: T, smallvec: &mut std::vec::Vec<T>) {
    if !smallvec.contains(&item) {
        smallvec.push(item);
    }
}

/// Writes the union of `other` and `smallvec` to the latter in O(N * M) time.
fn inplace_union<T: Eq>(other: Vec<T>, smallvec: &mut std::vec::Vec<T>) {
    for item in other {
        push_unique(item, smallvec);
    }
}

/// Writes the union of `other` and `smallvec` to the latter in O(N * M) time.
/// If an element in `other` has the same identity as an element in `smallvec`,
/// the elements are merged. Otherwise, the element is added to `smallvec`.
fn merge_by_identity<T: Merge>(
    other: Vec<T>,
    smallvec: &mut std::vec::Vec<T>,
    identity: impl Fn(&T, &T) -> bool,
) -> MergeResult {
    for item in other {
        if let Some(existing) = smallvec.iter_mut().find(|e| identity(e, &item)) {
            item.merge(existing)?;
        } else {
            smallvec.push(item);
        }
    }
    Ok(())
}

impl<T: Merge> Merge for Option<T> {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// If both `self` and `into` are Some, the underlying values are merged.
    /// Otherwise, the result is whichever value is Some, or None if neither is.
    fn merge(self, into: &mut Self) -> MergeResult {
        match self {
            Some(src) => match into.as_mut() {
                Some(dst) => src.merge(dst),
                None => {
                    *into = Some(src);
                    Ok(())
                }
            },
            None => Ok(()),
        }
    }
}

impl<K, V, I: IntoIterator<Item = (K, V)>> Merge<indexmap::IndexMap<K, V>> for I
where
    K: Eq + std::hash::Hash,
    V: Merge,
{
    /// Merges a sequence of key-value pairs into a map.
    ///
    /// If a key is present in both `self` and `into`, the corresponding values are merged.
    /// Otherwise, the entry from `self` is inserted into `into`.
    ///
    /// This implementation implies `indexmap::IndexMap<K,V>` is `Merge` for all `V: Merge`,
    /// because IndexMap is `IntoIterator`. We use `IntoIterator` to allow literal sequences of
    /// key-value pairs to be merged into a map.
    fn merge(self, into: &mut indexmap::IndexMap<K, V>) -> MergeResult {
        for (key, value) in self {
            match into.entry(key) {
                indexmap::map::Entry::Vacant(e) => {
                    e.insert(value);
                }
                indexmap::map::Entry::Occupied(mut e) => match value.merge(e.get_mut()) {
                    Ok(()) => {}
                    Err(message) => {
                        return Err(message);
                    }
                },
            }
        }
        Ok(())
    }
}

impl Merge for ZngurType {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// PRECONDITION: `self.ty == into.ty`.
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.ty != into.ty {
            panic!(
                "Attempt to merge different types: {} and {}",
                self.ty, into.ty
            );
        }

        if let (Some(layout1), Some(layout2)) = &(self.layout, into.layout)
            && layout1 != layout2
        {
            return Err(MergeFailure::Conflict(
                "Conflicting layout policy found".to_string(),
                (ConflictSource::Layout, ConflictSource::Layout),
            ));
        } else {
            into.layout = into.layout.or(self.layout);
        }

        // TODO: We need to improve the architecture around checking parsing, semantic, and other types of errors.
        if self.cpp_ref.is_some() && into.layout != Some(LayoutPolicy::ZERO_SIZED_TYPE) {
            return Err(MergeFailure::Conflict(
                "cpp_ref implies a zero sized stack allocated type".to_string(),
                (ConflictSource::CppRef, ConflictSource::Layout),
            ));
        }

        self.cpp_heap_allocated
            .merge(&mut into.cpp_heap_allocated)?;
        self.cpp_ref.merge(&mut into.cpp_ref)?;
        self.cpp_stack_owned.merge(&mut into.cpp_stack_owned)?;

        inplace_union(self.wellknown_traits, &mut into.wellknown_traits);
        merge_by_identity(self.methods, &mut into.methods, |a, b| {
            a.data.name == b.data.name
        })?;
        if into.constructor.is_none() {
            into.constructor = self.constructor;
        } else if self.constructor != into.constructor {
            return Err(MergeFailure::Conflict(
                "Duplicate constructor found".to_string(),
                (
                    ConflictSource::Constructor(
                        self.constructor.as_ref().map(|c| c.inputs.clone()),
                    ),
                    ConflictSource::Constructor(
                        into.constructor.as_ref().map(|c| c.inputs.clone()),
                    ),
                ),
            ));
        }
        into.exhaustive = self.exhaustive && into.exhaustive;
        merge_by_identity(self.variants, &mut into.variants, |a, b| a.name == b.name)?;
        merge_by_identity(self.fields, &mut into.fields, |a, b| a.name == b.name)?;

        Ok(())
    }
}

impl Merge for ZngurTrait {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// PRECONDITION: `self.tr == into.tr`.
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.tr != into.tr {
            panic!(
                "Attempt to merge different traits: {} and {}",
                self.tr, into.tr
            );
        }

        inplace_union(self.methods, &mut into.methods);

        Ok(())
    }
}

impl Merge for CppHeapAllocated {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// There is no meaningful way to merge different CppHeapAllocated values, but we
    /// allow merging the same CppHeapAllocated value from different sources.
    fn merge(self, into: &mut Self) -> MergeResult {
        if self != *into {
            return Err(MergeFailure::Conflict(
                "Cpp heap allocated mismatch".to_string(),
                (
                    ConflictSource::CppHeapAllocated,
                    ConflictSource::CppHeapAllocated,
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for CppRef {
    /// Writes the partial union of `self` and `into` to the latter.
    ///
    /// There is no meaningful way to merge different CppRefs, but we allow
    /// merging the same CppRef from different sources.
    fn merge(self, into: &mut Self) -> MergeResult {
        if self != *into {
            return Err(MergeFailure::Conflict(
                "Cpp ref mismatch".to_string(),
                (ConflictSource::CppRef, ConflictSource::CppRef),
            ));
        }
        Ok(())
    }
}

impl Merge for CppStackOwned {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self != *into {
            return Err(MergeFailure::Conflict(
                "Cpp stack owned mismatch".to_string(),
                (ConflictSource::CppStackOwned, ConflictSource::CppStackOwned),
            ));
        }
        Ok(())
    }
}

impl Merge<ZngurSpec> for ZngurType {
    /// Merges a type into a specification's type list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        if let Some(existing) = into.types.iter_mut().find(|t| t.ty == self.ty) {
            self.merge(existing)?;
        } else {
            into.types.push(self);
        }
        Ok(())
    }
}

impl Merge for ZngurMethodDetails {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self != *into {
            return Err(MergeFailure::Conflict(
                "Method mismatch".to_string(),
                (
                    ConflictSource::Method(self.data.clone()),
                    ConflictSource::Method(into.data.clone()),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for ZngurConstructor {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self != *into {
            return Err(MergeFailure::Conflict(
                "Constructor mismatch".to_string(),
                (
                    ConflictSource::Constructor(Some(self.inputs.clone())),
                    ConflictSource::Constructor(Some(into.inputs.clone())),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge for ZngurVariant {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self.name != into.name {
            panic!(
                "Attempt to merge different variants: {} and {}",
                self.name, into.name
            );
        }
        merge_by_identity(self.fields, &mut into.fields, |a, b| a.name == b.name)?;
        into.exhaustive = self.exhaustive && into.exhaustive;
        Ok(())
    }
}

impl Merge for ZngurField {
    fn merge(self, into: &mut Self) -> MergeResult {
        if self != *into {
            return Err(MergeFailure::Conflict(
                "Field mismatch".to_string(),
                (
                    ConflictSource::Field(self.name.clone()),
                    ConflictSource::Field(into.name.clone()),
                ),
            ));
        }
        Ok(())
    }
}

impl Merge<ZngurSpec> for ZngurTrait {
    /// Merges a trait into a specification's trait list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        [(self.tr.clone(), self)].merge(&mut into.traits)
    }
}

impl Merge<ZngurSpec> for ZngurFn {
    /// Merges a function into a specification's function list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        push_unique(self, &mut into.funcs);
        Ok(())
    }
}

impl Merge<ZngurSpec> for ZngurExternCppFn {
    /// Merges an extern C++ function into a specification's C++ function list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        push_unique(self, &mut into.extern_cpp_funcs);
        Ok(())
    }
}

impl Merge<ZngurSpec> for ZngurExternCppImpl {
    /// Merges an extern C++ implementation into a specification's C++ implementation list.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        push_unique(self, &mut into.extern_cpp_impls);
        Ok(())
    }
}

impl Merge<ZngurSpec> for AdditionalIncludes {
    /// Merges #include directives into a specification's additional includes string.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        into.additional_includes.0 += self.0.as_str();
        Ok(())
    }
}

impl Merge<ZngurSpec> for ConvertPanicToException {
    /// Merges a CPtE flag into a specification's CPtE flag.
    fn merge(self, into: &mut ZngurSpec) -> MergeResult {
        // TODO: There's an architectural decision here.
        // I'd like to encode the "unimportability" of CPTE here, rather than in the parser.
        // But checking this requires knowledge of the parse depth, which seems inappropriate
        // to pass through here.
        into.convert_panic_to_exception.0 |= self.0;
        Ok(())
    }
}
