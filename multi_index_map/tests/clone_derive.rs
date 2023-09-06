#![cfg_attr(feature = "trivial_bounds", feature(trivial_bounds))]
#![cfg(feature = "trivial_bounds")]
use multi_index_map::MultiIndexMap;

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct TestNonPrimitiveType(u64);

#[derive(MultiIndexMap, Clone, Debug)]
struct TestElement {
    #[multi_index(hashed_unique)]
    field1: TestNonPrimitiveType,
    field2: String,
}

#[test]
fn should_compile() {
    let mut map = MultiIndexTestElementMap::default();

    // check that formatting produces non empty strings
    assert!(!format!("{:?}", map._field1_index).is_empty());
    assert!(!format!("{:?}", map._store).is_empty());
    assert!(!format!("{:?}", map).is_empty());

    let elem1 = TestElement {
        field1: TestNonPrimitiveType(42),
        field2: "ElementOne".to_string(),
    };
    map.insert(elem1);

    let mut new_map = map.clone();

    assert_eq!(new_map.len(), 1);

    let elem1 = new_map.remove_by_field1(&TestNonPrimitiveType(42)).unwrap();
    assert_eq!(elem1.field1.0, 42);
    assert_eq!(elem1.field2, "ElementOne");
    assert_eq!(new_map.len(), 0);
}
