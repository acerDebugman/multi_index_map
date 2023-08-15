use ::convert_case::Casing;
use ::quote::{format_ident, quote};
use ::syn::{Field, Visibility};

use crate::index_attributes::{Ordering, Uniqueness};

// For each indexed field generate a TokenStream representing the lookup table for that field
// Each lookup table maps it's index to a position in the backing storage,
// or multiple positions in the backing storage in the non-unique indexes.
pub(crate) fn generate_lookup_tables<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
) -> impl Iterator<Item = ::proc_macro2::TokenStream> + 'a {
    fields.iter().map(|(f, ordering, uniqueness)| {
        let index_name = format_ident!("_{}_index", f.ident.as_ref().unwrap());
        let ty = &f.ty;

        match uniqueness {
            Uniqueness::Unique => match ordering {
                Ordering::Hashed => quote! {
                    #index_name: ::multi_index_map::rustc_hash::FxHashMap<#ty, usize>,
                },
                Ordering::Ordered => quote! {
                    #index_name: ::std::collections::BTreeMap<#ty, usize>,
                },
            },
            Uniqueness::NonUnique => match ordering {
                Ordering::Hashed => quote! {
                    #index_name: ::multi_index_map::rustc_hash::FxHashMap<#ty, ::std::collections::BTreeSet<usize>>,
                },
                Ordering::Ordered => quote! {
                    #index_name: ::std::collections::BTreeMap<#ty, ::std::collections::BTreeSet<usize>>,
                },
            },
        }
    })
}

// For each indexed field generate a TokenStream representing initializing the lookup table.
// Used in `with_capacity` initialization
// If lookup table data structures support `with_capacity`, change `default()` and `new()` calls to
//   `with_capacity(n)`
pub(crate) fn generate_lookup_table_init<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
) -> impl Iterator<Item = ::proc_macro2::TokenStream> + 'a {
    fields.iter().map(|(f, ordering, _uniqueness)| {
        let index_name = format_ident!("_{}_index", f.ident.as_ref().unwrap());

        match ordering {
            Ordering::Hashed => quote! {
                #index_name: ::multi_index_map::rustc_hash::FxHashMap::default(),
            },
            Ordering::Ordered => quote! {
                #index_name: ::std::collections::BTreeMap::new(),
            },
        }
    })
}

// For each indexed field generate a TokenStream representing reserving capacity in the lookup table.
// Used in `reserve`
// Currently `BTreeMap::extend_reserve()` is nightly-only and uses the trait default implementation, which does nothing.
// Once this is implemented and stabilized, we will use it here to reserve capacity.
pub(crate) fn generate_lookup_table_reserve<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
) -> impl Iterator<Item = ::proc_macro2::TokenStream> + 'a {
    fields.iter().map(|(f, ordering, _uniqueness)| {
        let index_name = format_ident!("_{}_index", f.ident.as_ref().unwrap());

        match ordering {
            Ordering::Hashed => quote! {
                self.#index_name.reserve(additional);
            },
            Ordering::Ordered => quote! {},
        }
    })
}

// For each indexed field generate a TokenStream representing shrinking the lookup table.
// Used in `shrink_to_fit`
// For consistency, HashMaps are shrunk to the capacity of the backing storage
// `BTreeMap` does not support shrinking.
pub(crate) fn generate_lookup_table_shrink<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
) -> impl Iterator<Item = ::proc_macro2::TokenStream> + 'a {
    fields.iter().map(|(f, ordering, _uniqueness)| {
        let index_name = format_ident!("_{}_index", f.ident.as_ref().unwrap());

        match ordering {
            Ordering::Hashed => quote! {
                self.#index_name.shrink_to_fit();
            },
            Ordering::Ordered => quote! {},
        }
    })
}

// For each indexed field generate a TokenStream representing inserting the position
//   in the backing storage to that field's lookup table
// Unique indexed fields just require a simple insert to the map,
//   whereas non-unique fields require inserting to the container of positions,
//   creating a new container if necessary.
pub(crate) fn generate_inserts<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
) -> impl Iterator<Item = ::proc_macro2::TokenStream> + 'a {
    fields.iter().map(|(f, _ordering, uniqueness)| {
        let field_name = f.ident.as_ref().unwrap();
        let field_name_string = field_name.to_string();
        let index_name = format_ident!("_{}_index", field_name);

        match uniqueness {
            Uniqueness::Unique => quote! {
                let orig_elem_idx = self.#index_name.insert(elem.#field_name.clone(), idx);
                if orig_elem_idx.is_some() {
                    panic!(
                        "Unable to insert element, uniqueness constraint violated on field '{}'",
                        #field_name_string
                    );
                }
            },
            Uniqueness::NonUnique => quote! {
                self.#index_name.entry(elem.#field_name.clone())
                    .or_insert(::std::collections::BTreeSet::new())
                    .insert(idx);
            },
        }
    })
}

// For each indexed field generate a TokenStream
//   representing the removal of an index from that field's lookup table.
// Used in remover. Run after an element is already removed from the backing storage.
// The removed element is given as `elem_orig`
// The index of the removed element in the backing storage before its removal is given as `idx`
// Remove idx from the lookup table:
//   - When the field is unique, check that the index is indeed idx,
//     then delete the corresponding key (elem_orig.#field_name) from the field
//   - When the field is non-unique, get a reference to the container that
//     contains all back storage indices under the same key (elem_orig.#field_name),
//     + If there are more than one indices in the container, remove idx from it
//     + If there are exactly one index in the container, then the index has to be idx,
//       remove the key from the lookup table
pub(crate) fn generate_removes(
    fields: &[(&Field, Ordering, Uniqueness)],
) -> Vec<::proc_macro2::TokenStream> {
    fields
        .iter()
        .map(|(f, _ordering, uniqueness)| {
            let field_name = f.ident.as_ref().unwrap();
            let field_name_string = field_name.to_string();
            let index_name = format_ident!("_{}_index", field_name);
            let error_msg = format!(
                concat!(
                    "Internal invariants broken, ",
                    "unable to find element in index '{}' despite being present in another"
                ),
                field_name_string
            );

            match uniqueness {
                Uniqueness::Unique => quote! {
                    let _removed_elem = self.#index_name.remove(&elem_orig.#field_name);
                },
                Uniqueness::NonUnique => quote! {
                    let key_to_remove = &elem_orig.#field_name;
                    if let Some(elems) = self.#index_name.get_mut(key_to_remove) {
                        if elems.len() > 1 {
                            if !elems.remove(&idx){
                                panic!(#error_msg);
                            }
                        } else {
                            self.#index_name.remove(key_to_remove);
                        }
                    }

                },
            }
        })
        .collect()
}

// For each indexed field generate a TokenStream representing the combined remove and insert from that
//   field's lookup table.
// Used in modifier. Run after an element is already modified in the backing storage.
// The element before the change is stored in `elem_orig`.
// The element after change is stored in reference `elem` (inside the backing storage).
// The index of `elem` in the backing storage is `idx`
// For each field, only make changes if elem.#field_name and elem_orig.#field_name are not equal
//   - When the field is unique, remove the old key and insert idx under the new key
//     (if new key already exists, panic!)
//   - When the field is non-unique, remove idx from the container associated with the old key
//     + if the container is empty after removal, remove the old key, and insert idx to the new key
//       (create a new container if necessary)
pub(crate) fn generate_modifies(
    fields: &[(&Field, Ordering, Uniqueness)],
) -> Vec<::proc_macro2::TokenStream> {
    fields.iter().map(|(f, _ordering, uniqueness)| {
        let field_name = f.ident.as_ref().unwrap();
        let field_name_string = field_name.to_string();
        let index_name = format_ident!("_{}_index", field_name);
        let error_msg = format!(
            concat!(
                "Internal invariants broken, ",
                "unable to find element in index '{}' despite being present in another"
            ),
            field_name_string
        );

        match uniqueness {
            Uniqueness::Unique => quote! {
                if elem.#field_name != elem_orig.#field_name {
                    let idx = self.#index_name.remove(&elem_orig.#field_name).expect(#error_msg);
                    let orig_elem_idx = self.#index_name.insert(elem.#field_name.clone(), idx);
                    if orig_elem_idx.is_some() {
                        panic!(
                            "Unable to insert element, uniqueness constraint violated on field '{}'",
                            #field_name_string
                        );
                    }
                }

            },
            Uniqueness::NonUnique => quote! {
                if elem.#field_name != elem_orig.#field_name {
                    let idxs = self.#index_name.get_mut(&elem_orig.#field_name).expect(#error_msg);
                    if idxs.len() > 1 {
                        if !(idxs.remove(&idx)) {
                            panic!(#error_msg);
                        }
                    } else {
                        self.#index_name.remove(&elem_orig.#field_name);
                    }
                    self.#index_name.entry(elem.#field_name.clone())
                        .or_insert(::std::collections::BTreeSet::new())
                        .insert(idx);
                }
            },
        }
    }).collect()
}

pub(crate) fn generate_clears<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
) -> impl Iterator<Item = ::proc_macro2::TokenStream> + 'a {
    fields.iter().map(|(f, _ordering, _uniqueness)| {
        let field_name = f.ident.as_ref().unwrap();
        let index_name = format_ident!("_{}_index", field_name);

        quote! {
            self.#index_name.clear();
        }
    })
}

// For each indexed field generate a TokenStream representing all the accessors
//   for the underlying storage via that field's lookup table.
pub(crate) fn generate_accessors<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
    map_name: &'a proc_macro2::Ident,
    element_name: &'a proc_macro2::Ident,
    removes: &'a [proc_macro2::TokenStream],
    modifies: &'a [proc_macro2::TokenStream],
) -> impl Iterator<Item = proc_macro2::TokenStream> + 'a {
    fields.iter().map(move |(f, ordering, uniqueness)| {
        let field_name = f.ident.as_ref().unwrap();
        let field_name_string = field_name.to_string();
        let field_vis = &f.vis;
        let index_name = format_ident!("_{}_index", field_name);
        let getter_name = format_ident!("get_by_{}", field_name);
        let mut_getter_name = format_ident!("get_mut_by_{}", field_name);
        let remover_name = format_ident!("remove_by_{}", field_name);
        let modifier_name = format_ident!("modify_by_{}", field_name);
        let iter_name = format_ident!(
            "{}{}Iter",
            map_name,
            field_name.to_string().to_case(::convert_case::Case::UpperCamel)
        );
        let iter_getter_name = format_ident!("iter_by_{}", field_name);
        let ty = &f.ty;

        // TokenStream representing the get_by_ accessor for this field.
        // For non-unique indexes we must go through all matching elements and find their positions,
        // in order to return a Vec of references to the backing storage.
        let getter = match uniqueness {
            Uniqueness::Unique => quote! {
                #field_vis fn #getter_name(&self, key: &#ty) -> Option<&#element_name> {
                    Some(&self._store[*self.#index_name.get(key)?])
                }
            },
            Uniqueness::NonUnique => quote! {
                #field_vis fn #getter_name(&self, key: &#ty) -> Vec<&#element_name> {
                    if let Some(idxs) = self.#index_name.get(key) {
                        let mut elem_refs = Vec::with_capacity(idxs.len());
                        for idx in idxs {
                            elem_refs.push(&self._store[*idx])
                        }
                        elem_refs
                    } else {
                        Vec::new()
                    }
                }
            },
        };

        // TokenStream representing the get_mut_by_ accessor for this field.
        let mut_getter = match uniqueness {
            Uniqueness::Unique => quote! {
                /// SAFETY:
                /// It is safe to mutate the non-indexed fields,
                /// however mutating any of the indexed fields will break the internal invariants.
                /// If the indexed fields need to be changed, the modify() method must be used.
                #field_vis unsafe fn #mut_getter_name(&mut self, key: &#ty) -> Option<&mut #element_name> {
                    Some(&mut self._store[*self.#index_name.get(key)?])
                }
            },
            Uniqueness::NonUnique => quote! {
                /// SAFETY:
                /// It is safe to mutate the non-indexed fields,
                /// however mutating any of the indexed fields will break the internal invariants.
                /// If the indexed fields need to be changed, the modify() method must be used.
                #field_vis unsafe fn #mut_getter_name(&mut self, key: &#ty) -> Vec<&mut #element_name> {
                    if let Some(idxs) = self.#index_name.get(key) {
                        let mut refs = Vec::with_capacity(idxs.len());
                        let mut mut_iter = self._store.iter_mut();
                        let mut last_idx: usize = 0;
                        for idx in idxs.iter() {
                            match mut_iter.nth(*idx - last_idx) {
                                Some(val) => {
                                    refs.push(val.1)
                                },
                                _ => {
                                    panic!(
                                        "Error getting mutable reference of non-unique field `{}` in getter.",
                                        #field_name_string
                                    );
                                }
                            }
                            last_idx = *idx + 1;
                        }
                        refs
                    } else {
                        Vec::new()
                    }
                }
            },
        };

        // TokenStream representing the remove_by_ accessor for this field.
        // For non-unique indexes we must go through all matching elements and find their positions,
        // in order to return a Vec elements from the backing storage.
        //      - get the back storage index(s)
        //      - mark the index(s) as unused in back storage
        //      - remove the index(s) from all fields
        //      - return the element(s)
        let remover = match uniqueness {
            Uniqueness::Unique => quote! {

                #field_vis fn #remover_name(&mut self, key: &#ty) -> Option<#element_name> {
                    let idx = self.#index_name.remove(key)?;
                    let elem_orig = self._store.remove(idx);
                    #(#removes)*
                    Some(elem_orig)
                }
            },
            Uniqueness::NonUnique => quote! {
                #field_vis fn #remover_name(&mut self, key: &#ty) -> Vec<#element_name> {
                    if let Some(idxs) = self.#index_name.remove(key) {
                        let mut elems = Vec::with_capacity(idxs.len());
                        for idx in idxs {
                            let elem_orig = self._store.remove(idx);
                            #(#removes)*
                            elems.push(elem_orig)
                        }
                        elems
                    } else {
                        Vec::new()
                    }
                }
            },
        };

        // TokenStream representing the modify_by_ accessor for this field.
        //      - obtain mutable reference (s) of the element
        //      - apply changes to the reference(s)
        //      - for each changed element, update all changed fields
        //      - return the modified item(s) as references
        let modifier = match uniqueness {
            Uniqueness::Unique => quote! {
                #field_vis fn #modifier_name(
                    &mut self,
                    key: &#ty,
                    f: impl FnOnce(&mut #element_name)
                ) -> Option<&#element_name> {
                    let idx = *self.#index_name.get(key)?;
                    let elem = &mut self._store[idx];
                    let elem_orig = elem.clone();
                    f(elem);
                    #(#modifies)*
                    Some(elem)
                }
            },
            Uniqueness::NonUnique => quote! {
                #field_vis fn #modifier_name(
                    &mut self,
                    key: &#ty,
                    f: impl Fn(&mut #element_name)
                ) -> Vec<&#element_name> {
                    let idxs = match self.#index_name.get(key) {
                        Some(container) => container.clone(),
                        _ => ::std::collections::BTreeSet::<usize>::new()
                    };
                    let mut refs = Vec::with_capacity(idxs.len());
                    let mut mut_iter = self._store.iter_mut();
                    let mut last_idx: usize = 0;
                    for idx in idxs {
                        match mut_iter.nth(idx - last_idx) {
                            Some(val) => {
                                let elem = val.1;
                                let elem_orig = elem.clone();
                                f(elem);
                                #(#modifies)*
                                refs.push(&*elem);
                            },
                            _ => {
                                panic!(
                                    "Error getting mutable reference of non-unique field `{}` in modifier.",
                                    #field_name_string
                                );
                            }
                        }
                        last_idx = idx + 1;
                    }
                    refs
                }
            },
        };

        let iterator_def = match ordering {
            Ordering::Hashed => quote! {
                #iter_name {
                    _store_ref: &self._store,
                    _iter: self.#index_name.iter(),
                    _inner_iter: None,
                }
            },
            Ordering::Ordered => quote! {
                #iter_name {
                    _store_ref: &self._store,
                    _iter: self.#index_name.iter(),
                    _iter_rev: self.#index_name.iter().rev(),
                    _inner_iter: None,
                }
            },
        };

        // Put all these TokenStreams together, and put a TokenStream representing the iter_by_ accessor
        //   on the end.
        quote! {
            #getter

            #mut_getter

            #remover

            #modifier

            #field_vis fn #iter_getter_name(&self) -> #iter_name {
                #iterator_def
            }
        }
    })
}

// For each indexed field generate a TokenStream representing the Iterator over the backing storage
//   via that field,
// such that the elements are accessed in an order defined by the index rather than the backing storage.
pub(crate) fn generate_iterators<'a>(
    fields: &'a [(&Field, Ordering, Uniqueness)],
    map_name: &'a proc_macro2::Ident,
    element_name: &'a proc_macro2::Ident,
) -> impl Iterator<Item = proc_macro2::TokenStream> + 'a {
    fields.iter().map(move |(f, ordering, uniqueness)| {
        let field_name = f.ident.as_ref().unwrap();
        let field_vis = &f.vis;
        let field_name_string = field_name.to_string();
        let error_msg = format!(
            "Internal invariants broken, found empty slice in non_unique index '{field_name_string}'"
        );
        let iter_name = format_ident!(
            "{}{}Iter",
            map_name,
            field_name.to_string().to_case(::convert_case::Case::UpperCamel)
        );
        let ty = &f.ty;

        // TokenStream representing the actual type of the iterator
        let iter_type = match uniqueness {
            Uniqueness::Unique => match ordering {
                Ordering::Hashed => quote! {::std::collections::hash_map::Iter<'a, #ty, usize>},
                Ordering::Ordered => quote! {::std::collections::btree_map::Iter<'a, #ty, usize>},
            },
            Uniqueness::NonUnique => match ordering {
                Ordering::Hashed => {
                    quote! {::std::collections::hash_map::Iter<'a, #ty, ::std::collections::BTreeSet::<usize>>}
                }
                Ordering::Ordered => {
                    quote! {::std::collections::btree_map::Iter<'a, #ty, ::std::collections::BTreeSet::<usize>>}
                }
            },
        };

        // TokenStream representing the logic for performing iteration.
        let iter_action = match uniqueness {
            Uniqueness::Unique => quote! { Some(&self._store_ref[*self._iter.next()?.1]) },
            Uniqueness::NonUnique => quote! {
                // If we have an inner_iter already, then get the next (optional) value from it.
                let inner_next = if let Some(inner_iter) = &mut self._inner_iter {
                    inner_iter.next()
                } else {
                    None
                };

                // If we have the next value, find it in the backing store.
                if let Some(next_index) = inner_next {
                    Some(&self._store_ref[*next_index])
                } else {
                    let hashmap_next = self._iter.next()?;
                    self._inner_iter = Some(Box::new(hashmap_next.1.iter()));
                    Some(&self._store_ref[*self._inner_iter.as_mut().unwrap().next().expect(#error_msg)])
                }
            },
        };

        let rev_iter_action = match uniqueness {
            Uniqueness::Unique => quote! {
                Some(&self._store_ref[*self._iter_rev.next()?.1])
            },
            Uniqueness::NonUnique => quote! {
                let inner_back = if let Some(inner_iter) = &mut self._inner_iter {
                    inner_iter.next_back()
                } else {
                    None
                };

                if let Some(back_index) = inner_back {
                    Some(&self._store_ref[*back_index])
                } else {
                    let hashmap_back = self._iter_rev.next()?;
                    self._inner_iter = Some(Box::new(hashmap_back.1.iter()));
                    Some(&self._store_ref[*self._inner_iter.as_mut().unwrap().next_back().expect(#error_msg)])
                }
            },
        };

        // TokenStream representing the iterator over each indexed field.
        // We have a different iterator type for each indexed field. Each one wraps the standard Iterator for
        //   that lookup table, but adds in a couple of things:
        // First we maintain a reference to the backing store, so we can return references to the elements we
        //   are interested in.
        // Second we maintain an optional inner_iter, only used for non-unique indexes. This is used to
        //   iterate through the container of matching elements for a given index value.
        // For ordered indices, we use _iter_rev to store a reversed iterator of the index field
        match ordering {
            // HashMap does not implement the DoubleEndedIterator trait,
            Ordering::Hashed => quote! {
                #field_vis struct #iter_name<'a> {
                    _store_ref: &'a ::multi_index_map::slab::Slab<#element_name>,
                    _iter: #iter_type,
                    _inner_iter: Option<Box<dyn ::std::iter::Iterator<Item=&'a usize> +'a>>,
                }

                impl<'a> Iterator for #iter_name<'a> {
                    type Item = &'a #element_name;
                    fn next(&mut self) -> Option<Self::Item> {
                        #iter_action
                    }
                }
            },
            Ordering::Ordered => quote! {
                #field_vis struct #iter_name<'a> {
                    _store_ref: &'a ::multi_index_map::slab::Slab<#element_name>,
                    _iter: #iter_type,
                    _iter_rev: ::std::iter::Rev<#iter_type>,
                    _inner_iter: Option<Box<dyn ::std::iter::DoubleEndedIterator<Item=&'a usize> +'a>>,
                }

                impl<'a> Iterator for #iter_name<'a> {
                    type Item = &'a #element_name;
                    fn next(&mut self) -> Option<Self::Item> {
                        #iter_action
                    }
                }

                impl<'a> DoubleEndedIterator for #iter_name<'a> {
                    fn next_back(&mut self) -> Option<Self::Item> {
                        #rev_iter_action
                    }
                }
            },
        }
    })
}

// Build the final output using quasi-quoting
#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_expanded(
    map_name: &proc_macro2::Ident,
    element_name: &proc_macro2::Ident,
    element_vis: &Visibility,
    inserts: impl Iterator<Item = proc_macro2::TokenStream>,
    accessors: impl Iterator<Item = proc_macro2::TokenStream>,
    iterators: impl Iterator<Item = proc_macro2::TokenStream>,
    clears: impl Iterator<Item = proc_macro2::TokenStream>,
    lookup_table_fields: impl Iterator<Item = proc_macro2::TokenStream>,
    lookup_table_fields_init: impl Iterator<Item = proc_macro2::TokenStream>,
    lookup_table_fields_shrink: impl Iterator<Item = proc_macro2::TokenStream>,
    lookup_table_fields_reserve: impl Iterator<Item = proc_macro2::TokenStream>,
) -> proc_macro2::TokenStream {
    let debug_impl = if cfg!(feature = "experimental") {
        quote! {
            #[allow(trivial_bounds)]
            impl ::core::fmt::Debug for #map_name where #element_name: ::core::fmt::Debug {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    f.debug_struct(stringify!(#map_name))
                        .field("_store", &self._store)
                        // #(#lookup_table_fields_debug)*
                        .finish()
                }
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #[derive(Default, Clone)]
        #element_vis struct #map_name {
            _store: ::multi_index_map::slab::Slab<#element_name>,
            #(#lookup_table_fields)*
        }

        #debug_impl

        impl #map_name {
            #element_vis fn with_capacity(n: usize) -> #map_name {
                #map_name {
                    _store: ::multi_index_map::slab::Slab::with_capacity(n),
                    #(#lookup_table_fields_init)*
                }
            }

            #element_vis fn capacity(&self) -> usize {
                self._store.capacity()
            }

            #element_vis fn len(&self) -> usize {
                self._store.len()
            }

            #element_vis fn is_empty(&self) -> bool {
                self._store.is_empty()
            }

            // reserving is slow. users are in control of when to reserve
            #element_vis fn reserve(&mut self, additional: usize) {
                self._store.reserve(additional);
                #(#lookup_table_fields_reserve)*
            }

            // shrinking is slow. users are in control of when to shrink
            #element_vis fn shrink_to_fit(&mut self) {
                self._store.shrink_to_fit();
                #(#lookup_table_fields_shrink)*
            }

            #element_vis fn insert(&mut self, elem: #element_name) {
                let idx = self._store.insert(elem);
                let elem = &self._store[idx];

                #(#inserts)*
            }

            #element_vis fn clear(&mut self) {
                self._store.clear();
                #(#clears)*
            }

            // Allow iteration directly over the backing storage
            #element_vis fn iter(&self) -> ::multi_index_map::slab::Iter<#element_name> {
                self._store.iter()
            }

            /// SAFETY:
            /// It is safe to mutate the non-indexed fields,
            /// however mutating any of the indexed fields will break the internal invariants.
            /// If the indexed fields need to be changed, the modify() method must be used.
            #element_vis unsafe fn iter_mut(&mut self) -> ::multi_index_map::slab::IterMut<#element_name> {
                self._store.iter_mut()
            }

            #(#accessors)*
        }

        #(#iterators)*

    }
}
