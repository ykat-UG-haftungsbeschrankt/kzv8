/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{BinaryTree, BinaryTreeNode, Color};

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
