/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::alloc::{GlobalAlloc, Layout};
#[allow(unused_imports)]
use core::ffi::c_void;
use core::ptr::null_mut;

use crate::kernel::lib::list::CircularListLinks;
use crate::kernel::memory::slab::SlabPage;
use crate::kernel::scheduler::sync::IrqSpinLock;

#[derive(Clone, Copy)]
struct Slab {
    allocation: SlabPage<4>,
    class_index: usize,
    links: CircularListLinks<usize>,
    active: bool,
}

impl Slab {
    const EMPTY: Self = Self {
        allocation: SlabPage::EMPTY,
        class_index: 0,
        links: CircularListLinks::new(Heap::NO_INDEX, Heap::NO_INDEX),
        active: false,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum IndexKind {
    Empty,
    Tombstone,
    Slab,
    Large,
    CachedLarge,
}

#[derive(Clone, Copy)]
struct IndexEntry {
    key: usize,
    value: usize,
    timestamp: usize,
    timestamp_previous: usize,
    timestamp_next: usize,
    tree_node: usize,
    kind: IndexKind,
}

impl IndexEntry {
    const EMPTY: Self = Self {
        key: 0,
        value: 0,
        timestamp: 0,
        timestamp_previous: Heap::NO_INDEX,
        timestamp_next: Heap::NO_INDEX,
        tree_node: Heap::NO_INDEX,
        kind: IndexKind::Empty,
    };

    const TOMBSTONE: Self = Self {
        kind: IndexKind::Tombstone,
        ..Self::EMPTY
    };
}

struct AddressIndex {
    address: usize,
    capacity: usize,
    length: usize,
    tombstones: usize,
    mapped_length: usize,
}

impl AddressIndex {
    const INITIAL_CAPACITY: usize = 16;

    const fn new() -> Self {
        Self {
            address: 0,
            capacity: 0,
            length: 0,
            tombstones: 0,
            mapped_length: 0,
        }
    }

    fn hash(key: usize) -> usize {
        let mut value = key as u64;
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value ^= value >> 27;
        value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
        (value ^ (value >> 31)) as usize
    }

    fn read(&self, slot: usize) -> IndexEntry {
        // SAFETY: the table storage is reserved for capacity entries and slots are bounded.
        unsafe { ((self.address as *const IndexEntry).add(slot)).read() }
    }

    fn write(&mut self, slot: usize, entry: IndexEntry) {
        // SAFETY: the table storage is reserved for capacity entries and slots are bounded.
        unsafe { ((self.address as *mut IndexEntry).add(slot)).write(entry) };
    }

    fn find(&self, key: usize) -> Option<usize> {
        if self.capacity == 0 {
            return None;
        }
        let mut slot = Self::hash(key) & (self.capacity - 1);
        for _ in 0..self.capacity {
            let entry = self.read(slot);
            if entry.kind == IndexKind::Empty {
                return None;
            }
            if entry.kind != IndexKind::Tombstone && entry.key == key {
                return Some(slot);
            }
            slot = (slot + 1) & (self.capacity - 1);
        }
        None
    }

    fn insertion_slot(&self, key: usize) -> Option<(usize, bool)> {
        let mut slot = Self::hash(key) & (self.capacity - 1);
        let mut first_tombstone = Heap::NO_INDEX;
        for _ in 0..self.capacity {
            let entry = self.read(slot);
            match entry.kind {
                IndexKind::Empty => {
                    return Some((
                        if first_tombstone == Heap::NO_INDEX {
                            slot
                        } else {
                            first_tombstone
                        },
                        false,
                    ));
                }
                IndexKind::Tombstone => {
                    if first_tombstone == Heap::NO_INDEX {
                        first_tombstone = slot;
                    }
                }
                _ if entry.key == key => return Some((slot, true)),
                _ => {}
            }
            slot = (slot + 1) & (self.capacity - 1);
        }
        (first_tombstone != Heap::NO_INDEX).then_some((first_tombstone, false))
    }

    fn insert_without_growth(&mut self, entry: IndexEntry) -> bool {
        let Some((slot, existing)) = self.insertion_slot(entry.key) else {
            return false;
        };
        if existing {
            self.write(slot, entry);
            return true;
        }
        if self.read(slot).kind == IndexKind::Tombstone {
            self.tombstones -= 1;
        }
        self.length += 1;
        self.write(slot, entry);
        true
    }

    fn resize(&mut self, new_capacity: usize) -> bool {
        let Some(bytes) = new_capacity.checked_mul(core::mem::size_of::<IndexEntry>()) else {
            return false;
        };
        let Some(mapped_length) = Heap::rounded_length(bytes) else {
            return false;
        };
        let Some(address) = crate::kernel::memory::r#virtual::reserve(
            mapped_length,
            Heap::PAGE_SIZE,
            2,
        ) else {
            return false;
        };

        let old_address = self.address;
        let old_capacity = self.capacity;
        let old_mapped_length = self.mapped_length;
        self.address = address;
        self.capacity = new_capacity;
        self.length = 0;
        self.tombstones = 0;
        self.mapped_length = mapped_length;
        for slot in 0..new_capacity {
            self.write(slot, IndexEntry::EMPTY);
        }
        for slot in 0..old_capacity {
            let entry =
                // SAFETY: the old table remains reserved until all live entries are copied.
                unsafe { ((old_address as *const IndexEntry).add(slot)).read() };
            if !matches!(entry.kind, IndexKind::Empty | IndexKind::Tombstone) {
                let _inserted = self.insert_without_growth(entry);
            }
        }
        if old_address != 0 {
            let _released =
                crate::kernel::memory::r#virtual::release(old_address, old_mapped_length);
        }
        true
    }

    fn grow(&mut self) -> bool {
        let new_capacity = if self.capacity == 0 {
            Self::INITIAL_CAPACITY
        } else {
            let Some(capacity) = self.capacity.checked_mul(2) else {
                return false;
            };
            capacity
        };
        self.resize(new_capacity)
    }

    fn insert(&mut self, entry: IndexEntry) -> bool {
        if self.capacity == 0 {
            if !self.grow() {
                return false;
            }
        } else if self.length + self.tombstones + 1 > self.capacity - self.capacity / 4 {
            let new_capacity = if self.length + 1 <= self.capacity / 2 {
                self.capacity
            } else {
                let Some(capacity) = self.capacity.checked_mul(2) else {
                    return false;
                };
                capacity
            };
            if !self.resize(new_capacity) {
                return false;
            }
        }
        self.insert_without_growth(entry)
    }

    fn get(&self, key: usize) -> Option<IndexEntry> {
        self.find(key).map(|slot| self.read(slot))
    }

    fn remove(&mut self, key: usize) -> Option<IndexEntry> {
        let slot = self.find(key)?;
        let entry = self.read(slot);
        self.write(slot, IndexEntry::TOMBSTONE);
        self.length -= 1;
        self.tombstones += 1;
        Some(entry)
    }

    fn reset(&mut self) {
        if self.address != 0 {
            let _released = crate::kernel::memory::r#virtual::release(
                self.address,
                self.mapped_length,
            );
        }
        *self = Self::new();
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SizeTreeLinks {
    left: usize,
    right: usize,
    parent: usize,
}

impl SizeTreeLinks {
    const fn new(left: usize, right: usize, parent: usize) -> Self {
        Self { left, right, parent }
    }

    const fn left(&self) -> usize {
        self.left
    }

    const fn right(&self) -> usize {
        self.right
    }

    const fn parent(&self) -> usize {
        self.parent
    }

    fn set_left(&mut self, left: usize) {
        self.left = left;
    }

    fn set_right(&mut self, right: usize) {
        self.right = right;
    }

    fn set_parent(&mut self, parent: usize) {
        self.parent = parent;
    }
}

#[derive(Clone, Copy)]
struct SizeNode {
    length: usize,
    address: usize,
    links: SizeTreeLinks,
    free_next: usize,
    height: usize,
    active: bool,
}

impl SizeNode {
    const EMPTY: Self = Self {
        length: 0,
        address: 0,
        links: SizeTreeLinks::new(Heap::NO_INDEX, Heap::NO_INDEX, Heap::NO_INDEX),
        free_next: Heap::NO_INDEX,
        height: 0,
        active: false,
    };
}

struct SizeIndex {
    address: usize,
    capacity: usize,
    mapped_length: usize,
    root: usize,
    free_head: usize,
}

impl SizeIndex {
    const INITIAL_CAPACITY: usize = 16;

    const fn new() -> Self {
        Self {
            address: 0,
            capacity: 0,
            mapped_length: 0,
            root: Heap::NO_INDEX,
            free_head: Heap::NO_INDEX,
        }
    }

    fn read(&self, node: usize) -> SizeNode {
        // SAFETY: nodes are read only after bounds and active-state checks.
        unsafe { ((self.address as *const SizeNode).add(node)).read() }
    }

    fn write(&mut self, node: usize, value: SizeNode) {
        // SAFETY: nodes are written only within the reserved node array.
        unsafe { ((self.address as *mut SizeNode).add(node)).write(value) };
    }

    fn grow(&mut self) -> bool {
        let new_capacity = if self.capacity == 0 {
            Self::INITIAL_CAPACITY
        } else {
            let Some(capacity) = self.capacity.checked_mul(2) else {
                return false;
            };
            capacity
        };
        let Some(bytes) = new_capacity.checked_mul(core::mem::size_of::<SizeNode>()) else {
            return false;
        };
        let Some(mapped_length) = Heap::rounded_length(bytes) else {
            return false;
        };
        let Some(address) = crate::kernel::memory::r#virtual::reserve(
            mapped_length,
            Heap::PAGE_SIZE,
            2,
        ) else {
            return false;
        };

        let old_address = self.address;
        let old_capacity = self.capacity;
        let old_mapped_length = self.mapped_length;
        if old_capacity != 0 {
            // SAFETY: both arrays are reserved, non-overlapping VM areas sized for their nodes.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    old_address as *const SizeNode,
                    address as *mut SizeNode,
                    old_capacity,
                );
            }
        }
        self.address = address;
        self.capacity = new_capacity;
        self.mapped_length = mapped_length;
        for node in (old_capacity..new_capacity).rev() {
            self.write(
                node,
                SizeNode {
                    free_next: self.free_head,
                    ..SizeNode::EMPTY
                },
            );
            self.free_head = node;
        }
        if old_address != 0 {
            let _released =
                crate::kernel::memory::r#virtual::release(old_address, old_mapped_length);
        }
        true
    }

    fn height(&self, node: usize) -> usize {
        if node == Heap::NO_INDEX {
            0
        } else {
            self.read(node).height
        }
    }

    fn update_height(&mut self, node: usize) {
        let mut value = self.read(node);
        value.height = 1
            + self
                .height(value.links.left())
                .max(self.height(value.links.right()));
        self.write(node, value);
    }

    fn balance_factor(&self, node: usize) -> isize {
        let value = self.read(node);
        self.height(value.links.left()) as isize - self.height(value.links.right()) as isize
    }

    fn replace_in_parent(&mut self, old: usize, new: usize) {
        let old_node = self.read(old);
        if old_node.links.parent() == Heap::NO_INDEX {
            self.root = new;
        } else {
            let mut parent = self.read(old_node.links.parent());
            if parent.links.left() == old {
                parent.links.set_left(new);
            } else {
                parent.links.set_right(new);
            }
            self.write(old_node.links.parent(), parent);
        }
        if new != Heap::NO_INDEX {
            let mut replacement = self.read(new);
            replacement.links.set_parent(old_node.links.parent());
            self.write(new, replacement);
        }
    }

    fn rotate_left(&mut self, node: usize) -> usize {
        let mut root = self.read(node);
        let pivot_index = root.links.right();
        let mut pivot = self.read(pivot_index);
        let parent = root.links.parent();
        let middle = pivot.links.left();
        self.replace_in_parent(node, pivot_index);
        root.links.set_parent(pivot_index);
        root.links.set_right(middle);
        pivot.links.set_parent(parent);
        pivot.links.set_left(node);
        self.write(node, root);
        self.write(pivot_index, pivot);
        if middle != Heap::NO_INDEX {
            let mut middle_node = self.read(middle);
            middle_node.links.set_parent(node);
            self.write(middle, middle_node);
        }
        self.update_height(node);
        self.update_height(pivot_index);
        pivot_index
    }

    fn rotate_right(&mut self, node: usize) -> usize {
        let mut root = self.read(node);
        let pivot_index = root.links.left();
        let mut pivot = self.read(pivot_index);
        let parent = root.links.parent();
        let middle = pivot.links.right();
        self.replace_in_parent(node, pivot_index);
        root.links.set_parent(pivot_index);
        root.links.set_left(middle);
        pivot.links.set_parent(parent);
        pivot.links.set_right(node);
        self.write(node, root);
        self.write(pivot_index, pivot);
        if middle != Heap::NO_INDEX {
            let mut middle_node = self.read(middle);
            middle_node.links.set_parent(node);
            self.write(middle, middle_node);
        }
        self.update_height(node);
        self.update_height(pivot_index);
        pivot_index
    }

    fn rebalance_from(&mut self, mut node: usize) {
        while node != Heap::NO_INDEX {
            self.update_height(node);
            let balance = self.balance_factor(node);
            let mut subtree_root = node;
            if balance > 1 {
                let left = self.read(node).links.left();
                if self.balance_factor(left) < 0 {
                    self.rotate_left(left);
                }
                subtree_root = self.rotate_right(node);
            } else if balance < -1 {
                let right = self.read(node).links.right();
                if self.balance_factor(right) > 0 {
                    self.rotate_right(right);
                }
                subtree_root = self.rotate_left(node);
            }
            node = self.read(subtree_root).links.parent();
        }
    }

    fn insert(&mut self, length: usize, address: usize) -> Option<usize> {
        let mut parent = Heap::NO_INDEX;
        let mut current = self.root;
        let mut insert_left = false;
        while current != Heap::NO_INDEX {
            parent = current;
            let value = self.read(current);
            if (length, address) < (value.length, value.address) {
                current = value.links.left();
                insert_left = true;
            } else if (length, address) > (value.length, value.address) {
                current = value.links.right();
                insert_left = false;
            } else {
                return None;
            }
        }
        if self.free_head == Heap::NO_INDEX && !self.grow() {
            return None;
        }
        let node = self.free_head;
        let free_node = self.read(node);
        self.free_head = free_node.free_next;
        self.write(
            node,
            SizeNode {
                length,
                address,
                links: SizeTreeLinks::new(Heap::NO_INDEX, Heap::NO_INDEX, parent),
                free_next: Heap::NO_INDEX,
                height: 1,
                active: true,
            },
        );
        if parent == Heap::NO_INDEX {
            self.root = node;
        } else {
            let mut parent_node = self.read(parent);
            if insert_left {
                parent_node.links.set_left(node);
            } else {
                parent_node.links.set_right(node);
            }
            self.write(parent, parent_node);
            self.rebalance_from(parent);
        }
        Some(node)
    }

    fn minimum(&self, mut node: usize) -> usize {
        while self.read(node).links.left() != Heap::NO_INDEX {
            node = self.read(node).links.left();
        }
        node
    }

    fn successor(&self, mut node: usize) -> usize {
        let right = self.read(node).links.right();
        if right != Heap::NO_INDEX {
            return self.minimum(right);
        }
        let mut parent = self.read(node).links.parent();
        while parent != Heap::NO_INDEX && self.read(parent).links.right() == node {
            node = parent;
            parent = self.read(parent).links.parent();
        }
        parent
    }

    fn best_fit(&self, length: usize, alignment: usize) -> Option<usize> {
        let mut current = self.root;
        let mut candidate = Heap::NO_INDEX;
        while current != Heap::NO_INDEX {
            let value = self.read(current);
            if value.length >= length {
                candidate = current;
                current = value.links.left();
            } else {
                current = value.links.right();
            }
        }
        while candidate != Heap::NO_INDEX {
            let value = self.read(candidate);
            if value.address % alignment == 0 {
                return Some(candidate);
            }
            candidate = self.successor(candidate);
        }
        None
    }

    fn remove(&mut self, node: usize) -> bool {
        if node >= self.capacity || !self.read(node).active {
            return false;
        }
        let value = self.read(node);
        let rebalance_start;
        if value.links.left() == Heap::NO_INDEX {
            self.replace_in_parent(node, value.links.right());
            rebalance_start = if value.links.parent() == Heap::NO_INDEX {
                value.links.right()
            } else {
                value.links.parent()
            };
        } else if value.links.right() == Heap::NO_INDEX {
            self.replace_in_parent(node, value.links.left());
            rebalance_start = if value.links.parent() == Heap::NO_INDEX {
                value.links.left()
            } else {
                value.links.parent()
            };
        } else {
            let successor = self.minimum(value.links.right());
            let mut successor_node = self.read(successor);
            if successor_node.links.parent() != node {
                rebalance_start = successor_node.links.parent();
                self.replace_in_parent(successor, successor_node.links.right());
                successor_node.links.set_right(value.links.right());
                let mut right = self.read(value.links.right());
                right.links.set_parent(successor);
                self.write(value.links.right(), right);
                self.write(successor, successor_node);
            } else {
                rebalance_start = successor;
            }
            self.replace_in_parent(node, successor);
            successor_node = self.read(successor);
            successor_node.links.set_left(value.links.left());
            let mut left = self.read(value.links.left());
            left.links.set_parent(successor);
            self.write(value.links.left(), left);
            self.write(successor, successor_node);
            self.update_height(successor);
        }

        self.write(
            node,
            SizeNode {
                free_next: self.free_head,
                ..SizeNode::EMPTY
            },
        );
        self.free_head = node;
        if rebalance_start != Heap::NO_INDEX {
            self.rebalance_from(rebalance_start);
        }
        true
    }

    fn reset(&mut self) {
        if self.address != 0 {
            let _released = crate::kernel::memory::r#virtual::release(
                self.address,
                self.mapped_length,
            );
        }
        *self = Self::new();
    }
}

pub(crate) struct HeapState {
    initialized: bool,
    slabs: [Slab; Heap::MAX_SLABS],
    free_slab_head: usize,
    class_heads: [usize; Heap::CLASS_SIZES.len()],
    address_index: AddressIndex,
    free_sizes: SizeIndex,
    freed_large_head: usize,
    freed_large_tail: usize,
    operation_timestamp: usize,
    last_cleanup_timestamp: usize,
    cached_large_bytes: usize,
}

impl HeapState {
    const fn new() -> Self {
        Self {
            initialized: false,
            slabs: [Slab::EMPTY; Heap::MAX_SLABS],
            free_slab_head: Heap::NO_INDEX,
            class_heads: [Heap::NO_INDEX; Heap::CLASS_SIZES.len()],
            address_index: AddressIndex::new(),
            free_sizes: SizeIndex::new(),
            freed_large_head: Heap::NO_INDEX,
            freed_large_tail: Heap::NO_INDEX,
            operation_timestamp: 0,
            last_cleanup_timestamp: 0,
            cached_large_bytes: 0,
        }
    }
}

pub(crate) struct Heap {
    pub(crate) state: IrqSpinLock<HeapState>,
}

#[cfg_attr(not(test), global_allocator)]
static GLOBAL_ALLOCATOR: Heap = Heap::new();

impl Heap {
    const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
    const CLASS_SIZES: [usize; 8] = [16, 32, 64, 128, 256, 512, 1024, 2048];
    const MAX_SLABS: usize = 8192;
    const NO_INDEX: usize = usize::MAX;
    const CLEANUP_INTERVAL: usize = 64;
    const CLEANUP_BUDGET: usize = 64;
    const CACHED_LARGE_TTL: usize = 4096;

    const fn new() -> Self {
        Self {
            state: IrqSpinLock::new(HeapState::new()),
        }
    }

    pub(crate) fn get_instance() -> &'static Self {
        &GLOBAL_ALLOCATOR
    }

    pub(crate) fn allocate(&self, heap: &mut HeapState, layout: Layout) -> *mut u8 {
        if !heap.initialized {
            return null_mut();
        }
        Self::advance_timestamp(heap);
        let size = layout.size().max(1);
        let required = size.max(layout.align());
        if let Some(class_index) = Self::CLASS_SIZES
            .iter()
            .position(|class| required <= *class)
        {
            return self.allocate_small(heap, class_index);
        }
        self.allocate_large(heap, size, layout.align())
    }

    fn allocate_small(&self, heap: &mut HeapState, class_index: usize) -> *mut u8 {
        let class_size = Self::CLASS_SIZES[class_index];
        let slot_count = Self::PAGE_SIZE / class_size;
        let mut slab_index = heap.class_heads[class_index];

        while slab_index != Self::NO_INDEX {
            let next = heap.slabs[slab_index].links.next();
            if heap.slabs[slab_index].allocation.used() >= slot_count {
                Self::unlink_slab(heap, slab_index);
                slab_index = next;
                continue;
            }
            if let Some(address) = heap.slabs[slab_index].allocation.allocate(class_size) {
                if heap.slabs[slab_index].allocation.used() == slot_count {
                    Self::unlink_slab(heap, slab_index);
                }
                return address as *mut u8;
            }
            slab_index = next;
        }

        let metadata_slot = heap.free_slab_head;
        if metadata_slot == Self::NO_INDEX {
            return null_mut();
        }
        let Some(page) =
            crate::kernel::memory::r#virtual::reserve(Self::PAGE_SIZE, Self::PAGE_SIZE, 2)
        else {
            return null_mut();
        };
        let mut allocation = SlabPage::new(page);
        let Some(address) = allocation.allocate(class_size) else {
            let _released = crate::kernel::memory::r#virtual::release(page, Self::PAGE_SIZE);
            return null_mut();
        };
        if !heap.address_index.insert(IndexEntry {
            key: page,
            value: metadata_slot,
            timestamp: 0,
            timestamp_previous: Self::NO_INDEX,
            timestamp_next: Self::NO_INDEX,
            tree_node: Self::NO_INDEX,
            kind: IndexKind::Slab,
        }) {
            let _released = crate::kernel::memory::r#virtual::release(page, Self::PAGE_SIZE);
            return null_mut();
        }
        heap.free_slab_head = heap.slabs[metadata_slot].links.next();
        let head = heap.class_heads[class_index];
        heap.slabs[metadata_slot] = Slab {
            allocation,
            class_index,
            links: CircularListLinks::new(Self::NO_INDEX, head),
            active: true,
        };
        if head != Self::NO_INDEX {
            heap.slabs[head].links.set_previous(metadata_slot);
        }
        heap.class_heads[class_index] = metadata_slot;
        address as *mut u8
    }

    fn link_slab(heap: &mut HeapState, index: usize) {
        let class_index = heap.slabs[index].class_index;
        let head = heap.class_heads[class_index];
        heap.slabs[index]
            .links
            .set_previous(Self::NO_INDEX);
        heap.slabs[index].links.set_next(head);
        if head != Self::NO_INDEX {
            heap.slabs[head].links.set_previous(index);
        }
        heap.class_heads[class_index] = index;
    }

    fn unlink_slab(heap: &mut HeapState, index: usize) {
        let slab = heap.slabs[index];
        let previous = slab.links.previous();
        let next = slab.links.next();
        if previous == Self::NO_INDEX {
            heap.class_heads[slab.class_index] = next;
        } else {
            heap.slabs[previous].links.set_next(next);
        }
        if next != Self::NO_INDEX {
            heap.slabs[next].links.set_previous(previous);
        }
        heap.slabs[index].links.reset(Self::NO_INDEX);
    }

    fn advance_timestamp(heap: &mut HeapState) {
        // Logical operation timestamps work before architecture timers are initialized.
        heap.operation_timestamp = heap.operation_timestamp.wrapping_add(1);
        if heap
            .operation_timestamp
            .wrapping_sub(heap.last_cleanup_timestamp)
            < Self::CLEANUP_INTERVAL
        {
            return;
        }
        heap.last_cleanup_timestamp = heap.operation_timestamp;
        Self::cleanup_cached_large(heap);
    }

    fn enqueue_cached_large(heap: &mut HeapState, address: usize) {
        let Some(slot) = heap.address_index.find(address) else {
            return;
        };
        let previous = heap.freed_large_tail;
        let mut entry = heap.address_index.read(slot);
        entry.timestamp_previous = previous;
        entry.timestamp_next = Self::NO_INDEX;
        heap.address_index.write(slot, entry);
        if previous == Self::NO_INDEX {
            heap.freed_large_head = address;
        } else if let Some(previous_slot) = heap.address_index.find(previous) {
            let mut previous_entry = heap.address_index.read(previous_slot);
            previous_entry.timestamp_next = address;
            heap.address_index.write(previous_slot, previous_entry);
        } else {
            heap.freed_large_head = address;
            heap.address_index.write(
                slot,
                IndexEntry {
                    timestamp_previous: Self::NO_INDEX,
                    ..entry
                },
            );
        }
        heap.freed_large_tail = address;
    }

    fn unlink_cached_large(heap: &mut HeapState, address: usize) {
        let Some(slot) = heap.address_index.find(address) else {
            return;
        };
        let mut entry = heap.address_index.read(slot);
        let previous = entry.timestamp_previous;
        let next = entry.timestamp_next;
        if previous == Self::NO_INDEX {
            if heap.freed_large_head == address {
                heap.freed_large_head = next;
            }
        } else if let Some(previous_slot) = heap.address_index.find(previous) {
            let mut previous_entry = heap.address_index.read(previous_slot);
            previous_entry.timestamp_next = next;
            heap.address_index.write(previous_slot, previous_entry);
        } else {
            heap.freed_large_head = next;
        }
        if next == Self::NO_INDEX {
            if heap.freed_large_tail == address {
                heap.freed_large_tail = previous;
            }
        } else if let Some(next_slot) = heap.address_index.find(next) {
            let mut next_entry = heap.address_index.read(next_slot);
            next_entry.timestamp_previous = previous;
            heap.address_index.write(next_slot, next_entry);
        } else {
            heap.freed_large_tail = previous;
        }
        entry.timestamp_previous = Self::NO_INDEX;
        entry.timestamp_next = Self::NO_INDEX;
        heap.address_index.write(slot, entry);
    }

    fn cleanup_cached_large(heap: &mut HeapState) {
        for _ in 0..Self::CLEANUP_BUDGET {
            let address = heap.freed_large_head;
            if address == Self::NO_INDEX {
                break;
            }
            let Some(entry) = heap.address_index.get(address) else {
                heap.freed_large_head = Self::NO_INDEX;
                heap.freed_large_tail = Self::NO_INDEX;
                break;
            };
            if entry.kind != IndexKind::CachedLarge {
                Self::unlink_cached_large(heap, address);
                continue;
            }
            if heap
                .operation_timestamp
                .wrapping_sub(entry.timestamp)
                < Self::CACHED_LARGE_TTL
            {
                break;
            }
            if crate::kernel::memory::r#virtual::release(entry.key, entry.value) {
                Self::unlink_cached_large(heap, address);
                if entry.tree_node != Self::NO_INDEX {
                    let _removed = heap.free_sizes.remove(entry.tree_node);
                    heap.cached_large_bytes =
                        heap.cached_large_bytes.saturating_sub(entry.value);
                }
                let _removed = heap.address_index.remove(entry.key);
            } else {
                Self::unlink_cached_large(heap, address);
                if let Some(slot) = heap.address_index.find(entry.key) {
                    let mut refreshed = heap.address_index.read(slot);
                    refreshed.timestamp = heap.operation_timestamp;
                    heap.address_index.write(slot, refreshed);
                    Self::enqueue_cached_large(heap, entry.key);
                }
            }
        }
    }

    fn rounded_length(length: usize) -> Option<usize> {
        length
            .checked_add(Self::PAGE_SIZE - 1)
            .map(|value| value & !(Self::PAGE_SIZE - 1))
    }

    fn allocate_large(&self, heap: &mut HeapState, size: usize, alignment: usize) -> *mut u8 {
        let Some(length) = Self::rounded_length(size) else {
            return null_mut();
        };
        let alignment = alignment.max(Self::PAGE_SIZE);

        while let Some(node) = heap.free_sizes.best_fit(length, alignment) {
            let cached = heap.free_sizes.read(node);
            let Some(slot) = heap.address_index.find(cached.address) else {
                let _removed = heap.free_sizes.remove(node);
                continue;
            };
            let mut entry = heap.address_index.read(slot);
            if entry.kind != IndexKind::CachedLarge
                || entry.value != cached.length
                || entry.tree_node != node
            {
                let _removed = heap.free_sizes.remove(node);
                continue;
            }
            if !heap.free_sizes.remove(node) {
                return null_mut();
            }
            Self::unlink_cached_large(heap, entry.key);
            entry = heap.address_index.read(slot);
            entry.kind = IndexKind::Large;
            entry.timestamp = 0;
            entry.timestamp_previous = Self::NO_INDEX;
            entry.timestamp_next = Self::NO_INDEX;
            entry.tree_node = Self::NO_INDEX;
            heap.address_index.write(slot, entry);
            heap.cached_large_bytes = heap.cached_large_bytes.saturating_sub(entry.value);
            return entry.key as *mut u8;
        }

        let Some(address) = crate::kernel::memory::r#virtual::reserve(length, alignment, 2) else {
            return null_mut();
        };
        if !heap.address_index.insert(IndexEntry {
            key: address,
            value: length,
            timestamp: 0,
            timestamp_previous: Self::NO_INDEX,
            timestamp_next: Self::NO_INDEX,
            tree_node: Self::NO_INDEX,
            kind: IndexKind::Large,
        }) {
            let _released = crate::kernel::memory::r#virtual::release(address, length);
            return null_mut();
        }
        address as *mut u8
    }

    pub(crate) fn deallocate(&self, heap: &mut HeapState, pointer: *mut u8) {
        if pointer.is_null() || !heap.initialized {
            return;
        }
        Self::advance_timestamp(heap);
        let address = pointer as usize;
        if let Some(slot) = heap.address_index.find(address) {
            let mut entry = heap.address_index.read(slot);
            if entry.kind == IndexKind::Large {
                entry.kind = IndexKind::CachedLarge;
                entry.timestamp = heap.operation_timestamp;
                entry.tree_node = Self::NO_INDEX;
                if let Some(node) = heap.free_sizes.insert(entry.value, entry.key) {
                    entry.tree_node = node;
                    heap.cached_large_bytes =
                        heap.cached_large_bytes.saturating_add(entry.value);
                    heap.address_index.write(slot, entry);
                } else if crate::kernel::memory::r#virtual::release(entry.key, entry.value) {
                    let _removed = heap.address_index.remove(entry.key);
                    return;
                } else {
                    heap.address_index.write(slot, entry);
                }
                Self::enqueue_cached_large(heap, entry.key);
                return;
            }
            if entry.kind == IndexKind::CachedLarge {
                return;
            }
        }

        let page = address & !(Self::PAGE_SIZE - 1);
        let Some(entry) = heap.address_index.get(page) else {
            return;
        };
        if entry.kind != IndexKind::Slab || entry.value >= Self::MAX_SLABS {
            return;
        }
        let index = entry.value;
        let slab = heap.slabs[index];
        if !slab.active || slab.allocation.address() != page {
            return;
        }
        let class_size = Self::CLASS_SIZES[slab.class_index];
        let slot_count = Self::PAGE_SIZE / class_size;
        let was_full = slab.allocation.used() == slot_count;
        if !heap.slabs[index]
            .allocation
            .deallocate(address, class_size)
        {
            return;
        }
        let unused = heap.slabs[index].allocation.used() == 0;
        if unused && crate::kernel::memory::r#virtual::release(page, Self::PAGE_SIZE) {
            Self::unlink_slab(heap, index);
            let _removed = heap.address_index.remove(page);
            heap.slabs[index] = Slab {
                allocation: SlabPage::EMPTY,
                links: CircularListLinks::new(Self::NO_INDEX, heap.free_slab_head),
                ..Slab::EMPTY
            };
            heap.free_slab_head = index;
        } else if was_full {
            Self::link_slab(heap, index);
        }
    }

    pub(crate) fn allocation_capacity(
        &self,
        heap: &HeapState,
        pointer: *const u8,
    ) -> Option<usize> {
        if pointer.is_null() || !heap.initialized {
            return None;
        }
        let address = pointer as usize;
        if let Some(entry) = heap.address_index.get(address) {
            match entry.kind {
                IndexKind::Large => return Some(entry.value),
                IndexKind::CachedLarge => return None,
                _ => {}
            }
        }

        let page = address & !(Self::PAGE_SIZE - 1);
        let entry = heap.address_index.get(page)?;
        if entry.kind != IndexKind::Slab || entry.value >= Self::MAX_SLABS {
            return None;
        }
        let slab = &heap.slabs[entry.value];
        if !slab.active || slab.allocation.address() != page {
            return None;
        }
        let class_size = Self::CLASS_SIZES[slab.class_index];
        slab
            .allocation
            .is_allocated(address, class_size)
            .then_some(class_size)
    }

    pub(crate) fn remaining(&self, heap: &HeapState) -> usize {
        let mut total = 0usize;
        for slab in &heap.slabs {
            if slab.active {
                let class_size = Self::CLASS_SIZES[slab.class_index];
                let slots = Self::PAGE_SIZE / class_size;
                total = total.saturating_add(
                    slots.saturating_sub(slab.allocation.used()) * class_size,
                );
            }
        }
        total.saturating_add(heap.cached_large_bytes)
    }

    pub(crate) fn initialize() {
        let mut heap = Self::get_instance().state.lock();
        heap.initialized = false;
        heap.address_index.reset();
        heap.free_sizes.reset();
        heap.slabs.fill(Slab::EMPTY);
        heap.free_slab_head = Self::NO_INDEX;
        for index in (0..Self::MAX_SLABS).rev() {
            let free_slab_head = heap.free_slab_head;
            heap.slabs[index]
                .links
                .set_next(free_slab_head);
            heap.free_slab_head = index;
        }
        heap.class_heads.fill(Self::NO_INDEX);
        heap.freed_large_head = Self::NO_INDEX;
        heap.freed_large_tail = Self::NO_INDEX;
        heap.operation_timestamp = 0;
        heap.last_cleanup_timestamp = 0;
        heap.cached_large_bytes = 0;
        heap.initialized = true;
    }

}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocate(&mut self.state.lock(), layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        self.deallocate(&mut self.state.lock(), pointer)
    }
}
