/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

/*
 * Ordered map; red-black balancing keeps each key path shallow:
 *
 *             [key]
 *            /     \
 *       [lower]   [higher]
 */

use alloc::boxed::Box;
use core::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    Red,
    Black,
}

impl Color {
    const fn opposite(self) -> Self {
        match self {
            Self::Red => Self::Black,
            Self::Black => Self::Red,
        }
    }
}

struct BinaryTreeNode<K, V> {
    left: Option<Box<Self>>,
    right: Option<Box<Self>>,
    color: Color,
    key: K,
    value: V,
}

impl<K, V> BinaryTreeNode<K, V> {
    const fn new(key: K, value: V) -> Self {
        Self {
            left: None,
            right: None,
            color: Color::Red,
            key,
            value,
        }
    }

    const fn key(&self) -> &K {
        &self.key
    }

    const fn value(&self) -> &V {
        &self.value
    }

    fn left(&self) -> Option<&Self> {
        self.left.as_deref()
    }

    fn right(&self) -> Option<&Self> {
        self.right.as_deref()
    }
}

/// A left-leaning red-black tree mapping unique keys to values.
pub(crate) struct BinaryTree<K, V> {
    root: Option<Box<BinaryTreeNode<K, V>>>,
    len: usize,
}

impl<K, V> BinaryTree<K, V> {
    pub(crate) const fn new() -> Self {
        Self { root: None, len: 0 }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn find_key_by<F>(&self, mut predicate: F) -> Option<&K>
    where
        F: FnMut(&K, &V) -> bool,
    {
        fn find<'a, K, V, F>(
            node: Option<&'a BinaryTreeNode<K, V>>,
            predicate: &mut F,
        ) -> Option<&'a K>
        where
            F: FnMut(&K, &V) -> bool,
        {
            let node = node?;
            find(node.left.as_deref(), predicate)
                .or_else(|| predicate(&node.key, &node.value).then_some(&node.key))
                .or_else(|| find(node.right.as_deref(), predicate))
        }

        find(self.root.as_deref(), &mut predicate)
    }

    pub(crate) fn clear(&mut self) {
        self.root = None;
        self.len = 0;
    }

    fn is_red(node: Option<&BinaryTreeNode<K, V>>) -> bool {
        node.is_some_and(|node| node.color == Color::Red)
    }

    fn flip_colors(node: &mut Box<BinaryTreeNode<K, V>>) {
        node.color = node.color.opposite();
        if let Some(left) = node.left.as_deref_mut() {
            left.color = left.color.opposite();
        }
        if let Some(right) = node.right.as_deref_mut() {
            right.color = right.color.opposite();
        }
    }

    fn rotate_left(mut root: Box<BinaryTreeNode<K, V>>) -> Box<BinaryTreeNode<K, V>> {
        let mut right = root.right.take().expect("left rotation requires a right child");
        root.right = right.left.take();
        let color = root.color;
        root.color = Color::Red;
        right.color = color;
        right.left = Some(root);
        right
    }

    fn rotate_right(mut root: Box<BinaryTreeNode<K, V>>) -> Box<BinaryTreeNode<K, V>> {
        let mut left = root.left.take().expect("right rotation requires a left child");
        root.left = left.right.take();
        let color = root.color;
        root.color = Color::Red;
        left.color = color;
        left.right = Some(root);
        left
    }

    fn fix_up_insert(mut root: Box<BinaryTreeNode<K, V>>) -> Box<BinaryTreeNode<K, V>> {
        if Self::is_red(root.right.as_deref()) && !Self::is_red(root.left.as_deref()) {
            root = Self::rotate_left(root);
        }
        if Self::is_red(root.left.as_deref())
            && root
                .left
                .as_deref()
                .is_some_and(|left| Self::is_red(left.left.as_deref()))
        {
            root = Self::rotate_right(root);
        }
        if Self::is_red(root.left.as_deref()) && Self::is_red(root.right.as_deref()) {
            Self::flip_colors(&mut root);
        }
        root
    }

    fn balance(mut root: Box<BinaryTreeNode<K, V>>) -> Box<BinaryTreeNode<K, V>> {
        if Self::is_red(root.right.as_deref()) {
            root = Self::rotate_left(root);
        }
        if Self::is_red(root.left.as_deref())
            && root
                .left
                .as_deref()
                .is_some_and(|left| Self::is_red(left.left.as_deref()))
        {
            root = Self::rotate_right(root);
        }
        if Self::is_red(root.left.as_deref()) && Self::is_red(root.right.as_deref()) {
            Self::flip_colors(&mut root);
        }
        root
    }

    fn move_red_left(mut root: Box<BinaryTreeNode<K, V>>) -> Box<BinaryTreeNode<K, V>> {
        Self::flip_colors(&mut root);
        if root
            .right
            .as_deref()
            .is_some_and(|right| Self::is_red(right.left.as_deref()))
        {
            let right = root.right.take().expect("red right subtree exists");
            root.right = Some(Self::rotate_right(right));
            root = Self::rotate_left(root);
            Self::flip_colors(&mut root);
        }
        root
    }

    fn move_red_right(mut root: Box<BinaryTreeNode<K, V>>) -> Box<BinaryTreeNode<K, V>> {
        Self::flip_colors(&mut root);
        if root
            .left
            .as_deref()
            .is_some_and(|left| Self::is_red(left.left.as_deref()))
        {
            root = Self::rotate_right(root);
            Self::flip_colors(&mut root);
        }
        root
    }
}

impl<K: Ord, V> BinaryTree<K, V> {
    /// Inserts a key/value pair, returning the old value when the key existed.
    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        let (root, previous, inserted) = Self::insert_node(self.root.take(), key, value);
        self.root = root;
        if let Some(root) = self.root.as_deref_mut() {
            root.color = Color::Black;
        }
        if inserted {
            self.len += 1;
        }
        previous
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        let mut current = self.root.as_deref();
        while let Some(node) = current {
            match key.cmp(node.key()) {
                Ordering::Less => current = node.left(),
                Ordering::Greater => current = node.right(),
                Ordering::Equal => return Some(node.value()),
            }
        }
        None
    }

    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let mut current = self.root.as_deref_mut();
        while let Some(node) = current {
            match key.cmp(node.key()) {
                Ordering::Less => current = node.left.as_deref_mut(),
                Ordering::Greater => current = node.right.as_deref_mut(),
                Ordering::Equal => return Some(&mut node.value),
            }
        }
        None
    }

    pub(crate) fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub(crate) fn first(&self) -> Option<(&K, &V)> {
        let mut current = self.root.as_deref()?;
        while let Some(left) = current.left() {
            current = left;
        }
        Some((current.key(), current.value()))
    }

    pub(crate) fn take_min(&mut self) -> Option<(K, V)> {
        let mut root = self.root.take()?;
        if !Self::is_red(root.left.as_deref()) && !Self::is_red(root.right.as_deref()) {
            root.color = Color::Red;
        }
        let (root, entry) = Self::take_min_node(root);
        self.root = root;
        if let Some(root) = self.root.as_deref_mut() {
            root.color = Color::Black;
        }
        self.len -= 1;
        Some(entry)
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let mut root = self.root.take()?;
        if !Self::is_red(root.left.as_deref()) && !Self::is_red(root.right.as_deref()) {
            root.color = Color::Red;
        }
        let (root, removed) = Self::remove_node(root, key);
        self.root = root;
        if let Some(root) = self.root.as_deref_mut() {
            root.color = Color::Black;
        }
        if removed.is_some() {
            self.len -= 1;
        }
        removed
    }

    /// Removes and returns the entry with the greatest key.
    pub(crate) fn take_max(&mut self) -> Option<(K, V)> {
        let mut root = self.root.take()?;
        if !Self::is_red(root.left.as_deref()) && !Self::is_red(root.right.as_deref()) {
            root.color = Color::Red;
        }
        let (root, entry) = Self::take_max_node(root);
        self.root = root;
        if let Some(root) = self.root.as_deref_mut() {
            root.color = Color::Black;
        }
        self.len -= 1;
        Some(entry)
    }

    fn insert_node(
        root: Option<Box<BinaryTreeNode<K, V>>>,
        key: K,
        value: V,
    ) -> (Option<Box<BinaryTreeNode<K, V>>>, Option<V>, bool) {
        let Some(mut root) = root else {
            return (Some(Box::new(BinaryTreeNode::new(key, value))), None, true);
        };
        match key.cmp(root.key()) {
            Ordering::Less => {
                let (left, previous, inserted) = Self::insert_node(root.left.take(), key, value);
                root.left = left;
                if inserted {
                    root = Self::fix_up_insert(root);
                }
                (Some(root), previous, inserted)
            }
            Ordering::Greater => {
                let (right, previous, inserted) = Self::insert_node(root.right.take(), key, value);
                root.right = right;
                if inserted {
                    root = Self::fix_up_insert(root);
                }
                (Some(root), previous, inserted)
            }
            Ordering::Equal => {
                let previous = Some(core::mem::replace(&mut root.value, value));
                (Some(root), previous, false)
            }
        }
    }

    fn remove_node(
        mut root: Box<BinaryTreeNode<K, V>>,
        key: &K,
    ) -> (Option<Box<BinaryTreeNode<K, V>>>, Option<V>) {
        let mut removed = None;
        if key.cmp(&root.key) == Ordering::Less {
            if root.left.is_some() {
                if !Self::is_red(root.left.as_deref())
                    && !root
                        .left
                        .as_deref()
                        .is_some_and(|left| Self::is_red(left.left.as_deref()))
                {
                    root = Self::move_red_left(root);
                }
                let left = root.left.take().expect("left deletion path exists");
                let (left, value) = Self::remove_node(left, key);
                root.left = left;
                removed = value;
            }
        } else {
            if Self::is_red(root.left.as_deref()) {
                root = Self::rotate_right(root);
            }
            if key.cmp(&root.key) == Ordering::Equal && root.right.is_none() {
                return (root.left.take(), Some(root.value));
            }
            if root.right.is_some() {
                if !Self::is_red(root.right.as_deref())
                    && !root
                        .right
                        .as_deref()
                        .is_some_and(|right| Self::is_red(right.left.as_deref()))
                {
                    root = Self::move_red_right(root);
                }
                if key.cmp(&root.key) == Ordering::Equal {
                    let right = root.right.take().expect("successor subtree exists");
                    let (right, (successor_key, successor_value)) = Self::take_min_node(right);
                    root.right = right;
                    root.key = successor_key;
                    removed = Some(core::mem::replace(&mut root.value, successor_value));
                } else {
                    let right = root.right.take().expect("right deletion path exists");
                    let (right, value) = Self::remove_node(right, key);
                    root.right = right;
                    removed = value;
                }
            }
        }
        (Some(Self::balance(root)), removed)
    }

    fn take_min_node(
        mut root: Box<BinaryTreeNode<K, V>>,
    ) -> (Option<Box<BinaryTreeNode<K, V>>>, (K, V)) {
        if root.left.is_none() {
            return (root.right.take(), (root.key, root.value));
        }
        if !Self::is_red(root.left.as_deref())
            && !root
                .left
                .as_deref()
                .is_some_and(|left| Self::is_red(left.left.as_deref()))
        {
            root = Self::move_red_left(root);
        }
        let left = root.left.take().expect("minimum path exists");
        let (left, entry) = Self::take_min_node(left);
        root.left = left;
        (Some(Self::balance(root)), entry)
    }

    fn take_max_node(
        mut root: Box<BinaryTreeNode<K, V>>,
    ) -> (Option<Box<BinaryTreeNode<K, V>>>, (K, V)) {
        if Self::is_red(root.left.as_deref()) {
            root = Self::rotate_right(root);
        }
        if root.right.is_none() {
            return (root.left.take(), (root.key, root.value));
        }
        if !Self::is_red(root.right.as_deref())
            && !root
                .right
                .as_deref()
                .is_some_and(|right| Self::is_red(right.left.as_deref()))
        {
            root = Self::move_red_right(root);
        }
        let right = root.right.take().expect("maximum path exists");
        let (right, entry) = Self::take_max_node(right);
        root.right = right;
        (Some(Self::balance(root)), entry)
    }
}

impl<K: Ord + Clone, V> BinaryTree<K, V> {
    /// Removes the smallest entry whose key is greater than or equal to `key`.
    pub(crate) fn take_greater_or_equal(&mut self, key: &K) -> Option<(K, V)> {
        let selected = self.least_key_at_or_above(key, true)?.clone();
        let value = self.remove(&selected)?;
        Some((selected, value))
    }

    /// Removes the smallest entry whose key is strictly greater than `key`.
    pub(crate) fn take_greater(&mut self, key: &K) -> Option<(K, V)> {
        let selected = self.least_key_at_or_above(key, false)?.clone();
        let value = self.remove(&selected)?;
        Some((selected, value))
    }

    /// Removes the greatest entry whose key is less than or equal to `key`.
    pub(crate) fn take_less_or_equal(&mut self, key: &K) -> Option<(K, V)> {
        let selected = self.greatest_key_at_or_below(key, true)?.clone();
        let value = self.remove(&selected)?;
        Some((selected, value))
    }

    /// Removes the greatest entry whose key is strictly less than `key`.
    pub(crate) fn take_less(&mut self, key: &K) -> Option<(K, V)> {
        let selected = self.greatest_key_at_or_below(key, false)?.clone();
        let value = self.remove(&selected)?;
        Some((selected, value))
    }

    fn least_key_at_or_above(&self, key: &K, inclusive: bool) -> Option<&K> {
        let mut current = self.root.as_deref();
        let mut candidate = None;
        while let Some(node) = current {
            match node.key.cmp(key) {
                Ordering::Less => current = node.right.as_deref(),
                Ordering::Equal if inclusive => return Some(&node.key),
                Ordering::Equal => current = node.right.as_deref(),
                Ordering::Greater => {
                    candidate = Some(&node.key);
                    current = node.left.as_deref();
                }
            }
        }
        candidate
    }

    fn greatest_key_at_or_below(&self, key: &K, inclusive: bool) -> Option<&K> {
        let mut current = self.root.as_deref();
        let mut candidate = None;
        while let Some(node) = current {
            match node.key.cmp(key) {
                Ordering::Greater => current = node.left.as_deref(),
                Ordering::Equal if inclusive => return Some(&node.key),
                Ordering::Equal => current = node.left.as_deref(),
                Ordering::Less => {
                    candidate = Some(&node.key);
                    current = node.right.as_deref();
                }
            }
        }
        candidate
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/btree.rs"]
mod tests;