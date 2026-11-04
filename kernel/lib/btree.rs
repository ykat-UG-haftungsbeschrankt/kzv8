use alloc::boxed::Box;
use core::cmp::Ordering;
use core::ptr::NonNull;

struct BinaryTreeNode<K, V> {
    left: Option<Box<Self>>,
    right: Option<Box<Self>>,
    parent: Option<NonNull<Self>>,
    key: K,
    value: V,
}

impl<K, V> BinaryTreeNode<K, V> {
    const fn new(key: K, value: V) -> Self {
        Self {
            left: None,
            right: None,
            parent: None,
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

    const fn parent(&self) -> Option<NonNull<Self>> {
        self.parent
    }
}

/// An unbalanced binary search tree mapping unique keys to values.
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

    pub(crate) fn clear(&mut self) {
        self.root = None;
        self.len = 0;
    }
}

impl<K: Ord, V> BinaryTree<K, V> {
    /// Inserts a key/value pair, returning the old value when the key existed.
    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        let (previous, inserted) = Self::insert_node(&mut self.root, None, key, value);
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

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let value = Self::remove_node(&mut self.root, key)?;
        self.len -= 1;
        Some(value)
    }

    /// Removes and returns the entry with the greatest key.
    pub(crate) fn take_max(&mut self) -> Option<(K, V)> {
        let entry = Self::take_max_node(&mut self.root)?;
        self.len -= 1;
        Some(entry)
    }

    /// Removes the smallest entry whose key is greater than or equal to `key`.
    pub(crate) fn take_greater_or_equal(&mut self, key: &K) -> Option<(K, V)> {
        let entry = Self::take_greater_node(&mut self.root, key, true)?;
        self.len -= 1;
        Some(entry)
    }

    /// Removes the smallest entry whose key is strictly greater than `key`.
    pub(crate) fn take_greater(&mut self, key: &K) -> Option<(K, V)> {
        let entry = Self::take_greater_node(&mut self.root, key, false)?;
        self.len -= 1;
        Some(entry)
    }

    /// Removes the greatest entry whose key is less than or equal to `key`.
    pub(crate) fn take_less_or_equal(&mut self, key: &K) -> Option<(K, V)> {
        let entry = Self::take_less_node(&mut self.root, key, true)?;
        self.len -= 1;
        Some(entry)
    }

    /// Removes the greatest entry whose key is strictly less than `key`.
    pub(crate) fn take_less(&mut self, key: &K) -> Option<(K, V)> {
        let entry = Self::take_less_node(&mut self.root, key, false)?;
        self.len -= 1;
        Some(entry)
    }

    fn insert_node(
        root: &mut Option<Box<BinaryTreeNode<K, V>>>,
        parent: Option<NonNull<BinaryTreeNode<K, V>>>,
        key: K,
        value: V,
    ) -> (Option<V>, bool) {
        let Some(node) = root.as_mut() else {
            let mut inserted = Box::new(BinaryTreeNode::new(key, value));
            inserted.parent = parent;
            *root = Some(inserted);
            return (None, true);
        };

        match key.cmp(node.key()) {
            Ordering::Less => {
                let parent = NonNull::from(node.as_mut());
                Self::insert_node(&mut node.left, Some(parent), key, value)
            }
            Ordering::Greater => {
                let parent = NonNull::from(node.as_mut());
                Self::insert_node(&mut node.right, Some(parent), key, value)
            }
            Ordering::Equal => (Some(core::mem::replace(&mut node.value, value)), false),
        }
    }

    fn remove_node(root: &mut Option<Box<BinaryTreeNode<K, V>>>, key: &K) -> Option<V> {
        let ordering = key.cmp(root.as_ref()?.key());
        match ordering {
            Ordering::Less => Self::remove_node(&mut root.as_mut()?.left, key),
            Ordering::Greater => Self::remove_node(&mut root.as_mut()?.right, key),
            Ordering::Equal => Self::take_root(root).map(|(_, value)| value),
        }
    }

    fn take_max_node(root: &mut Option<Box<BinaryTreeNode<K, V>>>) -> Option<(K, V)> {
        if root.as_ref()?.right.is_some() {
            return Self::take_max_node(&mut root.as_mut()?.right);
        }
        Self::take_root(root)
    }

    fn take_greater_node(
        root: &mut Option<Box<BinaryTreeNode<K, V>>>,
        key: &K,
        inclusive: bool,
    ) -> Option<(K, V)> {
        match key.cmp(root.as_ref()?.key()) {
            Ordering::Greater => Self::take_greater_node(&mut root.as_mut()?.right, key, inclusive),
            Ordering::Equal if inclusive => Self::take_root(root),
            Ordering::Equal => Self::take_greater_node(&mut root.as_mut()?.right, key, inclusive),
            Ordering::Less => {
                let candidate = {
                    let node = root.as_mut()?;
                    Self::take_greater_node(&mut node.left, key, inclusive)
                };
                if candidate.is_some() {
                    candidate
                } else {
                    Self::take_root(root)
                }
            }
        }
    }

    fn take_less_node(
        root: &mut Option<Box<BinaryTreeNode<K, V>>>,
        key: &K,
        inclusive: bool,
    ) -> Option<(K, V)> {
        match key.cmp(root.as_ref()?.key()) {
            Ordering::Less => Self::take_less_node(&mut root.as_mut()?.left, key, inclusive),
            Ordering::Equal if inclusive => Self::take_root(root),
            Ordering::Equal => Self::take_less_node(&mut root.as_mut()?.left, key, inclusive),
            Ordering::Greater => {
                let candidate = {
                    let node = root.as_mut()?;
                    Self::take_less_node(&mut node.right, key, inclusive)
                };
                if candidate.is_some() {
                    candidate
                } else {
                    Self::take_root(root)
                }
            }
        }
    }

    fn take_root(root: &mut Option<Box<BinaryTreeNode<K, V>>>) -> Option<(K, V)> {
        let mut removed = root.take()?;
        let parent = removed.parent;
        let left = removed.left.take();
        let right = removed.right.take();

        *root = match (left, right) {
            (None, None) => None,
            (Some(mut child), None) | (None, Some(mut child)) => {
                child.parent = parent;
                Some(child)
            }
            (Some(left), Some(right)) => {
                let mut right = Some(right);
                let mut successor = Self::take_min_node(&mut right);
                successor.left = Some(left);
                successor.right = right;
                successor.parent = parent;

                let successor_pointer = NonNull::from(successor.as_mut());
                if let Some(left) = successor.left.as_deref_mut() {
                    left.parent = Some(successor_pointer);
                }
                if let Some(right) = successor.right.as_deref_mut() {
                    right.parent = Some(successor_pointer);
                }
                Some(successor)
            }
        };

        Some((removed.key, removed.value))
    }

    fn take_min_node(root: &mut Option<Box<BinaryTreeNode<K, V>>>) -> Box<BinaryTreeNode<K, V>> {
        if root.as_ref().expect("non-empty subtree").left.is_some() {
            return Self::take_min_node(&mut root.as_mut().expect("non-empty subtree").left);
        }

        let mut minimum = root.take().expect("non-empty subtree");
        let parent = minimum.parent();
        let mut replacement = minimum.right.take();
        if let Some(replacement) = replacement.as_deref_mut() {
            replacement.parent = parent;
        }
        *root = replacement;
        minimum.parent = None;
        minimum
    }
}