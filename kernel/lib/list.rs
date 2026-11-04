use alloc::boxed::Box;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CircularListLinks<T> {
    previous: T,
    next: T,
}

impl<T> CircularListLinks<T> {
    pub(crate) const fn new(previous: T, next: T) -> Self {
        Self { previous, next }
    }
}

impl<T: Copy> CircularListLinks<T> {
    pub(crate) const fn previous(&self) -> T {
        self.previous
    }

    pub(crate) const fn next(&self) -> T {
        self.next
    }

    pub(crate) fn set_previous(&mut self, previous: T) {
        self.previous = previous;
    }

    pub(crate) fn set_next(&mut self, next: T) {
        self.next = next;
    }

    pub(crate) fn reset(&mut self, sentinel: T) {
        self.previous = sentinel;
        self.next = sentinel;
    }
}

pub(crate) struct ListNode<T> {
    value: T,
    previous: Option<NonNull<Self>>,
    next: Option<NonNull<Self>>,
    owned_next: Option<Box<Self>>,
}

impl<T> ListNode<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            value,
            previous: None,
            next: None,
            owned_next: None,
        }
    }

    pub(crate) fn into_value(self: Box<Self>) -> T {
        debug_assert!(self.previous.is_none() && self.next.is_none() && self.owned_next.is_none());
        self.value
    }
}

impl<T> Deref for ListNode<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for ListNode<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

// SAFETY: linked pointers only refer to nodes owned by the list, which moves
// as a unit; detached nodes have their links cleared before being returned.
unsafe impl<T: Send> Send for ListNode<T> {}

/// A circular ring of caller-allocated nodes; list operations do not allocate.
pub(crate) struct CircularList<T> {
    head: Option<Box<ListNode<T>>>,
    tail: Option<NonNull<ListNode<T>>>,
    len: usize,
}

// SAFETY: nodes are owned by this list, and their addresses remain stable when
// the list moves because each node is separately allocated by its caller.
unsafe impl<T: Send> Send for CircularList<T> {}

impl<T> CircularList<T> {
    pub(crate) const fn new() -> Self {
        Self {
            head: None,
            tail: None,
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn push_back(&mut self, mut node: Box<ListNode<T>>) {
        debug_assert!(node.previous.is_none() && node.next.is_none() && node.owned_next.is_none());
        let node_pointer = NonNull::from(node.as_mut());

        if self.len == 0 {
            node.previous = Some(node_pointer);
            node.next = Some(node_pointer);
            self.head = Some(node);
            self.tail = Some(node_pointer);
            self.len = 1;
            return;
        }

        let head_pointer = NonNull::from(self.head.as_deref_mut().expect("non-empty list"));
        let tail_pointer = self.tail.expect("non-empty list");
        node.previous = Some(tail_pointer);
        node.next = Some(head_pointer);
        // SAFETY: the list exclusively owns these nodes, and node_pointer is a
        // detached box that remains at a stable address when linked.
        unsafe {
            (*tail_pointer.as_ptr()).next = Some(node_pointer);
            (*head_pointer.as_ptr()).previous = Some(node_pointer);
            (*tail_pointer.as_ptr()).owned_next = Some(node);
        }
        self.tail = Some(node_pointer);
        self.len += 1;
    }

    pub(crate) fn push_front(&mut self, mut node: Box<ListNode<T>>) {
        debug_assert!(node.previous.is_none() && node.next.is_none() && node.owned_next.is_none());
        let node_pointer = NonNull::from(node.as_mut());

        if self.len == 0 {
            node.previous = Some(node_pointer);
            node.next = Some(node_pointer);
            self.head = Some(node);
            self.tail = Some(node_pointer);
            self.len = 1;
            return;
        }

        let head_pointer = NonNull::from(self.head.as_deref_mut().expect("non-empty list"));
        let tail_pointer = self.tail.expect("non-empty list");
        node.previous = Some(tail_pointer);
        node.next = Some(head_pointer);
        node.owned_next = self.head.take();
        // SAFETY: the list exclusively owns these nodes, and node_pointer is a
        // detached box that remains at a stable address when linked.
        unsafe {
            (*tail_pointer.as_ptr()).next = Some(node_pointer);
            (*head_pointer.as_ptr()).previous = Some(node_pointer);
        }
        self.head = Some(node);
        self.len += 1;
    }

    pub(crate) fn pop_front(&mut self) -> Option<Box<ListNode<T>>> {
        let mut head = self.head.take()?;
        if self.len == 1 {
            self.tail = None;
            self.len = 0;
            head.previous = None;
            head.next = None;
            return Some(head);
        }

        let mut new_head = head.owned_next.take().expect("non-empty list has successor");
        let new_head_pointer = NonNull::from(new_head.as_mut());
        let tail_pointer = self.tail.expect("non-empty list");
        // SAFETY: both pointers refer to nodes still owned by this list.
        unsafe {
            (*tail_pointer.as_ptr()).next = Some(new_head_pointer);
            (*new_head_pointer.as_ptr()).previous = Some(tail_pointer);
        }
        self.head = Some(new_head);
        self.len -= 1;
        head.previous = None;
        head.next = None;
        Some(head)
    }

    pub(crate) fn front(&self) -> Option<&T> {
        self.head.as_deref().map(|node| &node.value)
    }

    pub(crate) fn front_mut(&mut self) -> Option<&mut T> {
        self.head.as_deref_mut().map(|node| &mut node.value)
    }

    pub(crate) fn iter(&self) -> Iter<'_, T> {
        Iter {
            next: self.head.as_deref().map(NonNull::from),
            remaining: self.len,
            marker: PhantomData,
        }
    }

    pub(crate) fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            next: self.head.as_deref_mut().map(NonNull::from),
            remaining: self.len,
            marker: PhantomData,
        }
    }

    pub(crate) fn position(&self, predicate: impl FnMut(&T) -> bool) -> Option<usize> {
        self.iter().position(predicate)
    }

    pub(crate) fn remove(&mut self, index: usize) -> Option<Box<ListNode<T>>> {
        if index >= self.len {
            return None;
        }
        if index == 0 {
            return self.pop_front();
        }

        let mut previous_pointer = self.node_at(index - 1)?;
        // SAFETY: index is valid, so the predecessor owns the requested node
        // through its forward ownership link.
        let mut removed = unsafe { previous_pointer.as_mut().owned_next.take()? };
        let next_pointer = removed.next.expect("linked node has successor");
        let is_tail = self.tail == Some(NonNull::from(removed.as_mut()));
        let owned_successor = removed.owned_next.take();
        // SAFETY: these pointers are adjacent nodes in this exclusively
        // borrowed ring; the removed node's box is detached from ownership.
        unsafe {
            previous_pointer.as_mut().owned_next = owned_successor;
            previous_pointer.as_mut().next = Some(next_pointer);
            (*next_pointer.as_ptr()).previous = Some(previous_pointer);
        }
        if is_tail {
            self.tail = Some(previous_pointer);
        }
        self.len -= 1;
        removed.previous = None;
        removed.next = None;
        Some(removed)
    }

    pub(crate) fn clear(&mut self) {
        let mut next = self.head.take();
        self.tail = None;
        self.len = 0;
        while let Some(mut node) = next {
            next = node.owned_next.take();
            node.previous = None;
            node.next = None;
        }
    }

    fn node_at(&self, index: usize) -> Option<NonNull<ListNode<T>>> {
        if index >= self.len {
            return None;
        }
        let mut current = NonNull::from(self.head.as_deref()?);
        for _ in 0..index {
            // SAFETY: current is a node in the live ring owned by self.
            current = unsafe { current.as_ref().next? };
        }
        Some(current)
    }
}

impl<T> Drop for CircularList<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

pub(crate) struct Iter<'a, T> {
    next: Option<NonNull<ListNode<T>>>,
    remaining: usize,
    marker: PhantomData<&'a ListNode<T>>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let node_pointer = self.next?;
        // SAFETY: the iterator's shared borrow keeps all nodes alive, and the
        // remaining count ensures each ring element is yielded only once.
        let node = unsafe { node_pointer.as_ref() };
        self.next = node.next;
        self.remaining -= 1;
        Some(&node.value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}

pub(crate) struct IterMut<'a, T> {
    next: Option<NonNull<ListNode<T>>>,
    remaining: usize,
    marker: PhantomData<&'a mut ListNode<T>>,
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let node_pointer = self.next?;
        // SAFETY: the iterator exclusively borrows the list, and the remaining
        // count visits each distinct node once before the ring wraps.
        let node = unsafe { &mut *node_pointer.as_ptr() };
        self.next = node.next;
        self.remaining -= 1;
        Some(&mut node.value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for IterMut<'_, T> {}