/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;

use super::HashMap;

#[test]
fn borrowed_lookup_removal_and_fallible_iteration_work() {
	let mut map = HashMap::<String, usize, 1>::new();
	assert_eq!(map.insert(String::from("user.label"), 17), Ok(None));
	assert_eq!(map.get("user.label"), Some(&17));

	let mut visited = 0;
	map.try_for_each(|key, value| {
		assert_eq!(key.as_str(), "user.label");
		assert_eq!(*value, 17);
		visited += 1;
		Ok::<(), ()>(())
	})
	.unwrap();
assert_eq!(visited, 1);

	assert_eq!(map.remove("user.label"), Some(17));
	assert!(map.is_empty());
}