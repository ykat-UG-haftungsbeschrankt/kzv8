/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

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
}

impl<T> ListNode<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            value,
            previous: None,
            next: None,
        }
    }

    pub(crate) const fn previous(&self) -> Option<NonNull<Self>> {
        self.previous
    }

    pub(crate) const fn next(&self) -> Option<NonNull<Self>> {
        self.next
    }

    pub(crate) fn into_value(self: Box<Self>) -> T {
        debug_assert!(self.previous.is_none() && self.next.is_none());
        self.value
    }

    /// # Safety
    /// `node` must be a live node owned by `list`.
    pub(crate) unsafe fn remove(list: &mut LinkedList<T>, node: NonNull<Self>) -> Option<Box<Self>> {
        let head_pointer = NonNull::from(list.head.as_deref_mut()?);
        let (previous, next) = unsafe {
            let node = node.as_ref();
            (node.previous?, node.next?)
        };

        if previous == node && next == node {
            if node != head_pointer {
                return None;
            }
            let mut removed = list.head.take()?;
            removed.previous = None;
            removed.next = None;
            return Some(removed);
        }

        // SAFETY: the caller guarantees that node belongs to this live ring.
        unsafe {
            (*previous.as_ptr()).next = Some(next);
            (*next.as_ptr()).previous = Some(previous);
        }

        let mut removed = if node == head_pointer {
            let removed = list.head.take()?;
            // SAFETY: every non-head node is owned by a raw Box pointer held in the ring.
            list.head = Some(unsafe { Box::from_raw(next.as_ptr()) });
            removed
        } else {
            // SAFETY: every non-head node is owned by a raw Box pointer held in the ring.
            unsafe { Box::from_raw(node.as_ptr()) }
        };
        removed.previous = None;
        removed.next = None;
        Some(removed)
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

/// A circular doubly linked list; operations do not allocate.
pub(crate) struct LinkedList<T> {
    head: Option<Box<ListNode<T>>>,
}

// SAFETY: nodes are owned by this list, and their addresses remain stable when
// the list moves because each node is separately allocated by its caller.
unsafe impl<T: Send> Send for LinkedList<T> {}

impl<T> LinkedList<T> {
    pub(crate) const fn new() -> Self {
        Self { head: None }
    }

    pub(crate) fn len(&self) -> usize {
        self.iter().count()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    pub(crate) fn push_back(&mut self, mut node: Box<ListNode<T>>) {
        debug_assert!(node.previous.is_none() && node.next.is_none());
        let node_pointer = NonNull::from(node.as_mut());

        if self.head.is_none() {
            node.previous = Some(node_pointer);
            node.next = Some(node_pointer);
            self.head = Some(node);
            return;
        }

        let head_pointer = NonNull::from(self.head.as_deref_mut().expect("non-empty list"));
        // SAFETY: head_pointer refers to the first node in this ring.
        let tail_pointer = unsafe {
            (*head_pointer.as_ptr())
                .previous
                .expect("non-empty ring has a tail")
        };
        node.previous = Some(tail_pointer);
        node.next = Some(head_pointer);
        let _ = Box::into_raw(node);
        // SAFETY: the ring owns both pointers and they are adjacent at the tail.
        unsafe {
            (*tail_pointer.as_ptr()).next = Some(node_pointer);
            (*head_pointer.as_ptr()).previous = Some(node_pointer);
        }
    }

    pub(crate) fn push_front(&mut self, mut node: Box<ListNode<T>>) {
        debug_assert!(node.previous.is_none() && node.next.is_none());
        let node_pointer = NonNull::from(node.as_mut());

        let Some(mut head) = self.head.take() else {
            node.previous = Some(node_pointer);
            node.next = Some(node_pointer);
            self.head = Some(node);
            return;
        };

        let head_pointer = NonNull::from(head.as_mut());
        // SAFETY: head_pointer refers to the first node in this ring.
        let tail_pointer = unsafe {
            (*head_pointer.as_ptr())
                .previous
                .expect("non-empty ring has a tail")
        };
        node.previous = Some(tail_pointer);
        node.next = Some(head_pointer);
        // SAFETY: these pointers refer to adjacent nodes in this ring.
        unsafe {
            (*tail_pointer.as_ptr()).next = Some(node_pointer);
            (*head_pointer.as_ptr()).previous = Some(node_pointer);
        }
        let _ = Box::into_raw(head);
        self.head = Some(node);
    }

    pub(crate) fn pop_front(&mut self) -> Option<Box<ListNode<T>>> {
        let head = NonNull::from(self.head.as_deref_mut()?);
        // SAFETY: head is the first live node owned by this list.
        unsafe { ListNode::remove(self, head) }
    }

    pub(crate) fn front(&self) -> Option<&T> {
        self.head.as_deref().map(|node| &node.value)
    }

    pub(crate) fn front_mut(&mut self) -> Option<&mut T> {
        self.head.as_deref_mut().map(|node| &mut node.value)
    }

    pub(crate) fn iter(&self) -> Iter<'_, T> {
        let head = self.head.as_deref().map(NonNull::from);
        Iter {
            head,
            next: head,
            marker: PhantomData,
        }
    }

    pub(crate) fn iter_mut(&mut self) -> IterMut<'_, T> {
        let head = self.head.as_deref_mut().map(NonNull::from);
        IterMut {
            head,
            next: head,
            marker: PhantomData,
        }
    }

    pub(crate) fn find(
        &self,
        mut predicate: impl FnMut(&T) -> bool,
    ) -> Option<NonNull<ListNode<T>>> {
        let head = NonNull::from(self.head.as_deref()?);
        let mut current = head;
        loop {
            // SAFETY: current is a node in the live ring owned by this list.
            let node = unsafe { current.as_ref() };
            if predicate(&node.value) {
                return Some(current);
            }
            let next = node.next.expect("linked node missing next pointer");
            if next == head {
                return None;
            }
            current = next;
        }
    }

    pub(crate) fn clear(&mut self) {
        let Some(mut head) = self.head.take() else {
            return;
        };
        let head_pointer = NonNull::from(head.as_mut());
        let mut current = head.next.expect("linked head missing next pointer");
        while current != head_pointer {
            // SAFETY: current is a non-head node owned by this ring.
            let next = unsafe {
                current
                    .as_ref()
                    .next
                    .expect("linked node missing next pointer")
            };
            // SAFETY: non-head nodes were converted from Box into raw pointers when linked.
            unsafe { drop(Box::from_raw(current.as_ptr())) };
            current = next;
        }
        head.previous = None;
        head.next = None;
    }
}

impl<T> Drop for LinkedList<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

pub(crate) struct Iter<'a, T> {
    head: Option<NonNull<ListNode<T>>>,
    next: Option<NonNull<ListNode<T>>>,
    marker: PhantomData<&'a ListNode<T>>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let node_pointer = self.next?;
        // SAFETY: the iterator's shared borrow keeps all nodes alive, and the
        // head pointer marks the end after one traversal of the ring.
        let node = unsafe { node_pointer.as_ref() };
        let next = node.next.expect("linked node missing next pointer");
        self.next = if Some(next) == self.head {
            None
        } else {
            Some(next)
        };
        Some(&node.value)
    }
}

pub(crate) struct IterMut<'a, T> {
    head: Option<NonNull<ListNode<T>>>,
    next: Option<NonNull<ListNode<T>>>,
    marker: PhantomData<&'a mut ListNode<T>>,
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        let node_pointer = self.next?;
        // SAFETY: the iterator's mutable list borrow keeps every ring node alive.
        let next = unsafe {
            node_pointer
                .as_ref()
                .next
                .expect("linked node missing next pointer")
        };
        self.next = if Some(next) == self.head {
            None
        } else {
            Some(next)
        };
        // SAFETY: the iterator exclusively borrows the list, and the head
        // pointer ensures each distinct node is visited once.
        let node = unsafe { &mut *node_pointer.as_ptr() };
        Some(&mut node.value)
    }
}