/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
	use super::{AddressIndex, UserAddress};

	#[test]
	fn resolves_and_rejects_stale_handles_after_reuse() {
		let mut index = AddressIndex::<usize, 1>::new();
		let first = index.insert(0x1000).unwrap();
		assert_eq!(index.get(first), Some(&0x1000));
		assert_eq!(index.get(UserAddress::from_raw(first.raw())), Some(&0x1000));
		assert_eq!(index.remove(first), Some(0x1000));
		assert_eq!(index.get(first), None);

		let second = index.insert(0x2000).unwrap();
		assert_ne!(first, second);
		assert_eq!(index.get(first), None);
		assert_eq!(index.get(second), Some(&0x2000));
	}

	#[test]
	fn rejects_invalid_handles_and_preserves_values_when_full() {
		let mut index = AddressIndex::<usize, 1>::new();
		assert_eq!(index.get(UserAddress::from_raw(0)), None);
		let _address = index.insert(0x1000).unwrap();
		assert_eq!(index.insert(0x2000), Err(0x2000));
		assert_eq!(index.len(), 1);
		assert_eq!(index.capacity(), 1);
	}
