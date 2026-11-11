//! Resolve the exported inventory against the source index, including refused points.
use super::*;

#[test]
fn every_exported_point_resolves_to_its_indexed_function() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = crate::build::wire_types(&root).unwrap();
    let (scanned, list) = scan(&root).unwrap();
    let indexed = all_points(&scanned, &list, &wire).unwrap();
    let targets = indexed
        .iter()
        .map(|(_, found)| found.target())
        .collect::<std::collections::HashSet<_>>();
    let points = points_with(&root, &wire).unwrap();
    assert!(!points.is_empty());
    for point in &points {
        assert!(targets.contains(&point.target), "{}", point.target);
    }
    let wrong = format!("{}_nonexistent", points[0].target);
    assert!(!targets.contains(&wrong));
}
