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

use alloc::alloc::alloc;
use alloc::boxed::Box;
use core::alloc::Layout;
use core::cmp::Ordering;
use core::ptr::NonNull;

/// Recomputes a copyable subtree summary from a node and its child summaries.
pub(crate) trait TreeAugmentation<K, V> {
    type Summary: Copy;

    fn summarize(
        key: &K,
        value: &V,
        left: Option<Self::Summary>,
        right: Option<Self::Summary>,
    ) -> Self::Summary;
}

pub(crate) struct NoAugmentation;

impl<K, V> TreeAugmentation<K, V> for NoAugmentation {
    type Summary = ();

    fn summarize(
        _key: &K,
        _value: &V,
        _left: Option<Self::Summary>,
        _right: Option<Self::Summary>,
    ) -> Self::Summary {
    }
}

fn try_box_new<T>(value: T) -> Result<Box<T>, T> {
    let layout = Layout::new::<T>();
    if layout.size() == 0 {
        return Ok(Box::new(value));
    }
    let Some(pointer) = NonNull::new(unsafe { alloc(layout).cast::<T>() }) else {
        return Err(value);
    };
    // SAFETY: pointer is aligned storage for one T allocated above.
    unsafe { pointer.as_ptr().write(value) };
    // SAFETY: pointer now contains an initialized T allocated by the global allocator.
    Ok(unsafe { Box::from_raw(pointer.as_ptr()) })
}

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

struct BinaryTreeNode<K, V, A: TreeAugmentation<K, V> = NoAugmentation> {
    left: Option<Box<Self>>,
    right: Option<Box<Self>>,
    color: Color,
    key: K,
    value: V,
    summary: A::Summary,
}

impl<K, V, A: TreeAugmentation<K, V>> BinaryTreeNode<K, V, A> {
    fn new(key: K, value: V) -> Self {
        let summary = A::summarize(&key, &value, None, None);
        Self {
            left: None,
            right: None,
            color: Color::Red,
            key,
            value,
            summary,
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
pub(crate) struct BinaryTree<K, V, A: TreeAugmentation<K, V> = NoAugmentation> {
    root: Option<Box<BinaryTreeNode<K, V, A>>>,
    len: usize,
}

impl<K, V, A: TreeAugmentation<K, V>> BinaryTree<K, V, A> {
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
        fn find<'a, K, V, A, F>(
            node: Option<&'a BinaryTreeNode<K, V, A>>,
            predicate: &mut F,
        ) -> Option<&'a K>
        where
            A: TreeAugmentation<K, V>,
            F: FnMut(&K, &V) -> bool,
        {
            let node = node?;
            find(node.left.as_deref(), predicate)
                .or_else(|| predicate(&node.key, &node.value).then_some(&node.key))
                .or_else(|| find(node.right.as_deref(), predicate))
        }

        find(self.root.as_deref(), &mut predicate)
    }

    pub(crate) fn for_each(&self, mut visit: impl FnMut(&K, &V)) {
        fn walk<K, V, A, F>(node: Option<&BinaryTreeNode<K, V, A>>, visit: &mut F)
        where
            A: TreeAugmentation<K, V>,
            F: FnMut(&K, &V),
        {
            let Some(node) = node else {
                return;
            };
            walk(node.left.as_deref(), visit);
            visit(&node.key, &node.value);
            walk(node.right.as_deref(), visit);
        }

        walk(self.root.as_deref(), &mut visit);
    }

    /// Visits entries in key order until the callback returns false.
    /// Returns true only if the traversal reached the end of the tree.
    pub(crate) fn visit_in_order_until(
        &self,
        mut visit: impl FnMut(&K, &V) -> bool,
    ) -> bool {
        fn walk<K, V, A, F>(node: Option<&BinaryTreeNode<K, V, A>>, visit: &mut F) -> bool
        where
            A: TreeAugmentation<K, V>,
            F: FnMut(&K, &V) -> bool,
        {
            let Some(node) = node else {
                return true;
            };
            walk(node.left.as_deref(), visit)
                && visit(&node.key, &node.value)
                && walk(node.right.as_deref(), visit)
        }

        walk(self.root.as_deref(), &mut visit)
    }

    /// Finds the first matching entry in key order, pruning subtrees rejected by `subtree_may_match`.
    /// The pruning predicate must be conservative: false must mean no entry in that subtree can match.
    pub(crate) fn find_first_by(
        &self,
        mut subtree_may_match: impl FnMut(&A::Summary) -> bool,
        mut matches: impl FnMut(&K, &V) -> bool,
    ) -> Option<(&K, &V)> {
        fn find<'a, K, V, A, MayMatch, Matches>(
            node: Option<&'a BinaryTreeNode<K, V, A>>,
            subtree_may_match: &mut MayMatch,
            matches: &mut Matches,
        ) -> Option<(&'a K, &'a V)>
        where
            A: TreeAugmentation<K, V>,
            MayMatch: FnMut(&A::Summary) -> bool,
            Matches: FnMut(&K, &V) -> bool,
        {
            let node = node?;
            if !subtree_may_match(&node.summary) {
                return None;
            }
            find(node.left.as_deref(), subtree_may_match, matches)
                .or_else(|| matches(&node.key, &node.value).then_some((&node.key, &node.value)))
                .or_else(|| find(node.right.as_deref(), subtree_may_match, matches))
        }

        find(
            self.root.as_deref(),
            &mut subtree_may_match,
            &mut matches,
        )
    }

    pub(crate) fn clear(&mut self) {
        self.root = None;
        self.len = 0;
    }

    fn is_red(node: Option<&BinaryTreeNode<K, V, A>>) -> bool {
        node.is_some_and(|node| node.color == Color::Red)
    }

    fn flip_colors(node: &mut Box<BinaryTreeNode<K, V, A>>) {
        node.color = node.color.opposite();
        if let Some(left) = node.left.as_deref_mut() {
            left.color = left.color.opposite();
        }
        if let Some(right) = node.right.as_deref_mut() {
            right.color = right.color.opposite();
        }
    }

    fn refresh(node: &mut BinaryTreeNode<K, V, A>) {
        node.summary = A::summarize(
            &node.key,
            &node.value,
            node.left.as_deref().map(|left| left.summary),
            node.right.as_deref().map(|right| right.summary),
        );
    }

    fn rotate_left(mut root: Box<BinaryTreeNode<K, V, A>>) -> Box<BinaryTreeNode<K, V, A>> {
        let mut right = root.right.take().expect("left rotation requires a right child");
        root.right = right.left.take();
        let color = root.color;
        root.color = Color::Red;
        right.color = color;
        right.left = Some(root);
        Self::refresh(right.left.as_deref_mut().expect("left rotation moved its root"));
        Self::refresh(&mut right);
        right
    }

    fn rotate_right(mut root: Box<BinaryTreeNode<K, V, A>>) -> Box<BinaryTreeNode<K, V, A>> {
        let mut left = root.left.take().expect("right rotation requires a left child");
        root.left = left.right.take();
        let color = root.color;
        root.color = Color::Red;
        left.color = color;
        left.right = Some(root);
        Self::refresh(left.right.as_deref_mut().expect("right rotation moved its root"));
        Self::refresh(&mut left);
        left
    }

    fn fix_up_insert(mut root: Box<BinaryTreeNode<K, V, A>>) -> Box<BinaryTreeNode<K, V, A>> {
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
        Self::refresh(&mut root);
        root
    }

    fn balance(mut root: Box<BinaryTreeNode<K, V, A>>) -> Box<BinaryTreeNode<K, V, A>> {
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
        Self::refresh(&mut root);
        root
    }

    fn move_red_left(mut root: Box<BinaryTreeNode<K, V, A>>) -> Box<BinaryTreeNode<K, V, A>> {
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

    fn move_red_right(mut root: Box<BinaryTreeNode<K, V, A>>) -> Box<BinaryTreeNode<K, V, A>> {
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

impl<K: Ord, V, A: TreeAugmentation<K, V>> BinaryTree<K, V, A> {
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

    /// Inserts without changing the tree if allocating a new node fails.
    pub(crate) fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, (K, V)> {
        if self.get(&key).is_some() {
            let (root, previous, inserted) = Self::insert_node(self.root.take(), key, value);
            self.root = root;
            if let Some(root) = self.root.as_deref_mut() {
                root.color = Color::Black;
            }
            debug_assert!(!inserted);
            return Ok(previous);
        }

        let node = try_box_new(BinaryTreeNode::<K, V, A>::new(key, value))
            .map_err(|node| (node.key, node.value))?;
        let mut root = match self.root.take() {
            Some(root) => Self::insert_boxed_node(root, node),
            None => node,
        };
        root.color = Color::Black;
        self.root = Some(root);
        self.len += 1;
        Ok(None)
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
        root: Option<Box<BinaryTreeNode<K, V, A>>>,
        key: K,
        value: V,
    ) -> (Option<Box<BinaryTreeNode<K, V, A>>>, Option<V>, bool) {
        let Some(mut root) = root else {
            return (
                Some(Box::new(BinaryTreeNode::<K, V, A>::new(key, value))),
                None,
                true,
            );
        };
        match key.cmp(root.key()) {
            Ordering::Less => {
                let (left, previous, inserted) = Self::insert_node(root.left.take(), key, value);
                root.left = left;
                if inserted {
                    root = Self::fix_up_insert(root);
                } else {
                    Self::refresh(&mut root);
                }
                (Some(root), previous, inserted)
            }
            Ordering::Greater => {
                let (right, previous, inserted) = Self::insert_node(root.right.take(), key, value);
                root.right = right;
                if inserted {
                    root = Self::fix_up_insert(root);
                } else {
                    Self::refresh(&mut root);
                }
                (Some(root), previous, inserted)
            }
            Ordering::Equal => {
                let previous = Some(core::mem::replace(&mut root.value, value));
                Self::refresh(&mut root);
                (Some(root), previous, false)
            }
        }
    }

    fn insert_boxed_node(
        mut root: Box<BinaryTreeNode<K, V, A>>,
        inserted: Box<BinaryTreeNode<K, V, A>>,
    ) -> Box<BinaryTreeNode<K, V, A>> {
        match inserted.key.cmp(&root.key) {
            Ordering::Less => {
                root.left = Some(match root.left.take() {
                    Some(left) => Self::insert_boxed_node(left, inserted),
                    None => inserted,
                });
            }
            Ordering::Greater => {
                root.right = Some(match root.right.take() {
                    Some(right) => Self::insert_boxed_node(right, inserted),
                    None => inserted,
                });
            }
            Ordering::Equal => unreachable!("prepared tree node key already exists"),
        }
        Self::fix_up_insert(root)
    }

    fn remove_node(
        mut root: Box<BinaryTreeNode<K, V, A>>,
        key: &K,
    ) -> (Option<Box<BinaryTreeNode<K, V, A>>>, Option<V>) {
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
        mut root: Box<BinaryTreeNode<K, V, A>>,
    ) -> (Option<Box<BinaryTreeNode<K, V, A>>>, (K, V)) {
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
        mut root: Box<BinaryTreeNode<K, V, A>>,
    ) -> (Option<Box<BinaryTreeNode<K, V, A>>>, (K, V)) {
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

impl<K: Ord + Clone, V, A: TreeAugmentation<K, V>> BinaryTree<K, V, A> {
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