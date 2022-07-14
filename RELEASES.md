Version 0.2.0 (2022-07-14)
==========================

- Add `hashed_non_unique` field attribute, with associated `insert_by_` and `iter_by_` accessors.
- Add initial test for `hashed_non_unique`.
- Ensure non-primitive types (ie. user-defined structs) are imported to the `multi_index` module to be used as indexes.