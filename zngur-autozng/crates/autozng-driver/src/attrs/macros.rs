macro_rules! __trait_cast_fn_pair {
    ($map:ident, $source:ty, $trt:path $(,)? ) => {
        (
            ::std::any::TypeId::of::<dyn $trt>(),
            // cast &dyn to *dyn and then to *const $source and re-borrow as ref to downcast
            // without `std::any::Any::downcast_ref`s `'static` constraint
            &$crate::attrs::AttrTargetCaster::<dyn $trt> {
                // SAFETY: The `AttrTargetCaster<dyn $trt>` ensures that $trt is implemented by $source
                // the caller of `cast_ref` ensures that `from` is actually a $source
                // and the contained lifetimes of &source remain valid
                cast_ref: |from| unsafe {
                    &*(from as *const dyn $crate::attrs::AttrTarget as *const $source)
                },
            },
        )
    };
}

macro_rules! __trait_cast_fn_pairs {
    ($map:ident, $source:ty $(, $trt:path)* $(,)?) => {
        &[
            $(
                $crate::attrs::macros::__trait_cast_fn_pair!($map, $source, $trt)
            ),*
        ]
    };
}

macro_rules! impl_attr_target {
    ( $source:ty $(, $trt:path)* $(,)?) => {
        impl $crate::attrs::AttrTarget for $source {
            fn caster(&self) -> $crate::attrs::AttrTargetCasterTable {

                const CASTER_MAP: & [(::std::any::TypeId, &dyn ::std::any::Any)] =
                   $crate::attrs::macros::__trait_cast_fn_pairs!(m, $source $(, $trt)*);

                $crate::attrs::AttrTargetCasterTable {
                    casters: CASTER_MAP,
                    type_id: ::std::any::TypeId::of::<$source>(),
                }
            }

            fn type_id(&self) -> ::std::any::TypeId {
                ::std::any::TypeId::of::<$source>()
            }
        }
    };
}

pub(crate) use __trait_cast_fn_pair;
pub(crate) use __trait_cast_fn_pairs;
pub(crate) use impl_attr_target;

macro_rules! impl_has_attrs {
    (@; $name: path { WithAttrKey |& $self:ident| $key_block:block $(, WithParents |& $parents_self: ident, $key_name: ident| $parents_block:block)? }  ) => {

        impl $crate::attrs::parse::storage::HasAttrs for $name {
            fn attr_key(& $self ) -> rustc_span::def_id::DefId {
                $key_block
            }
        }

        impl  $crate::attrs::parse::storage::HasParentAttrs for $name {
        $(
            fn parent_attrs(& $parents_self, $key_name: rustc_span::def_id::DefId) -> Option<Vec<::std::rc::Rc<dyn $crate::attrs::DynAttr>>> {
                $parents_block
            }
        )?
        }

    };
    (@; $name: path { UseCrateDef  $(, WithParents |& $parents_self: ident, $key_name: ident| $parents_block:block)? } ) => {

        impl  $crate::attrs::parse::storage::HasAttrs for $name {
            fn attr_key(&self) -> rustc_span::def_id::DefId {
                self.def_id().attr_key()
            }
        }
        impl  $crate::attrs::parse::storage::HasParentAttrs for $name {
        $(
            fn parent_attrs(& $parents_self, $key_name: rustc_span::def_id::DefId) -> Option<Vec<::std::rc::Rc<dyn $crate::attrs::DynAttr>>> {
                $parents_block
            }
        )?
        }

    };
    (@; $name: path { $( WithParents |& $self: ident, $key_name: ident| $parents_block:block)? } ) => {

        impl $crate::attrs::parse::storage::HasParentAttrs for $name {
        $(
            fn parent_attrs(& $self, $key_name: rustc_span::def_id::DefId) -> Option<Vec<::std::rc::Rc<dyn $crate::attrs::DynAttr>>> {
                $parents_block
            }
        )?
        }

    };
    ( $( $name: path  { $($details:tt)* } ),+ $(,)?) => {
        $(
            $crate::attrs::attrs_for::impl_has_attrs!{@; $name { $($details)* }  }
        )+
    };
}
pub(crate) use impl_has_attrs;
