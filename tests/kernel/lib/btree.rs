/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{BinaryTree, BinaryTreeNode, Color, TreeAugmentation};

    struct MaximumValue;

    fn summary_over_threshold(summary: &usize) -> bool {
        *summary > 900
    }

    fn value_over_threshold(_key: &usize, value: &usize) -> bool {
        *value > 900
    }

    impl TreeAugmentation<usize, usize> for MaximumValue {
        type Summary = usize;

        fn summarize(
            _key: &usize,
            value: &usize,
            left: Option<Self::Summary>,
            right: Option<Self::Summary>,
        ) -> Self::Summary {
            left.unwrap_or(*value).max(*value).max(right.unwrap_or(*value))
        }
    }

    fn assert_red_black(node: Option<&BinaryTreeNode<usize, usize>>) -> (usize, usize) {
        let Some(node) = node else {
            return (1, 0);
        };
        if node.color == Color::Red {
            assert_ne!(node.left.as_deref().map(|left| left.color), Some(Color::Red));
            assert_ne!(node.right.as_deref().map(|right| right.color), Some(Color::Red));
        }
        let (left_black_height, left_height) = assert_red_black(node.left.as_deref());
        let (right_black_height, right_height) = assert_red_black(node.right.as_deref());
        assert_eq!(left_black_height, right_black_height);
        let black_node = if node.color == Color::Black { 1 } else { 0 };
        (
            left_black_height + black_node,
            1 + left_height.max(right_height),
        )
    }

    fn assert_valid(tree: &BinaryTree<usize, usize>) {
        if let Some(root) = tree.root.as_deref() {
            assert_eq!(root.color, Color::Black);
        }
        let (black_height, height) = assert_red_black(tree.root.as_deref());
        assert!(height <= 2 * black_height);
    }

    fn assert_augmented_valid(
        tree: &BinaryTree<usize, usize, MaximumValue>,
    ) -> (usize, usize, Option<usize>) {
        fn inspect(
            node: Option<&BinaryTreeNode<usize, usize, MaximumValue>>,
        ) -> (usize, usize, Option<usize>) {
            let Some(node) = node else {
                return (1, 0, None);
            };
            if node.color == Color::Red {
                assert_ne!(node.left.as_deref().map(|left| left.color), Some(Color::Red));
                assert_ne!(node.right.as_deref().map(|right| right.color), Some(Color::Red));
            }
            let (left_black_height, left_height, left_maximum) =
                inspect(node.left.as_deref());
            let (right_black_height, right_height, right_maximum) =
                inspect(node.right.as_deref());
            assert_eq!(left_black_height, right_black_height);
            let maximum = node.value
                .max(left_maximum.unwrap_or(0))
                .max(right_maximum.unwrap_or(0));
            assert_eq!(node.summary, maximum);
            (
                left_black_height + usize::from(node.color == Color::Black),
                1 + left_height.max(right_height),
                Some(maximum),
            )
        }

        if let Some(root) = tree.root.as_deref() {
            assert_eq!(root.color, Color::Black);
        }
        let result = inspect(tree.root.as_deref());
        assert!(result.1 <= 2 * result.0);
        result
    }

    #[test]
    fn sorted_insertions_and_removals_preserve_red_black_invariants() {
        let mut tree = BinaryTree::new();
        for key in 0..1024 {
            tree.insert(key, key);
        }
        assert_valid(&tree);

        for key in (0..1024).step_by(2) {
            assert_eq!(tree.remove(&key), Some(key));
            assert_valid(&tree);
        }
        for key in 100..200 {
            if let Some((selected, value)) = tree.take_greater_or_equal(&key) {
                assert_eq!(selected, value);
            }
            assert_valid(&tree);
        }
        while let Some((key, value)) = tree.take_max() {
            assert_eq!(key, value);
            assert_valid(&tree);
        }
        assert!(tree.is_empty());
    }

    #[test]
    fn augmented_search_tracks_insert_replace_and_remove() {
        let mut tree = BinaryTree::<usize, usize, MaximumValue>::new();
        for key in (0..512).rev() {
            assert_eq!(tree.try_insert(key, key * 2), Ok(None));
            assert_augmented_valid(&tree);
        }
        assert_eq!(
            tree.find_first_by(summary_over_threshold, value_over_threshold)
                .map(|(key, value)| (*key, *value)),
            Some((451, 902))
        );
        assert_eq!(tree.try_insert(451, 4), Ok(Some(902)));
        assert_eq!(tree.try_insert(450, 1000), Ok(Some(900)));
        assert_eq!(
            tree.find_first_by(summary_over_threshold, value_over_threshold)
                .map(|(key, value)| (*key, *value)),
            Some((450, 1000))
        );
        assert_augmented_valid(&tree);
        for key in 0..512 {
            tree.remove(&key);
            assert_augmented_valid(&tree);
        }
        assert!(
            tree.find_first_by(summary_over_threshold, value_over_threshold)
                .is_none()
        );
    }
