/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::cmp::Ordering;
use core::ops::Range;

const NO_NODE: u32 = u32::MAX;

struct IntervalNode<T> {
	range: Range<usize>,
	value: Option<T>,
	max_end: usize,
	left: u32,
	right: u32,
	free_next: u32,
	height: u8,
}

impl<T> IntervalNode<T> {
	const fn empty() -> Self {
		Self {
			range: Range { start: 0, end: 0 },
			value: None,
			max_end: 0,
			left: NO_NODE,
			right: NO_NODE,
			free_next: NO_NODE,
			height: 0,
		}
	}

	fn new(range: Range<usize>, value: T) -> Self {
		let max_end = range.end;
		Self {
			range,
			value: Some(value),
			max_end,
			left: NO_NODE,
			right: NO_NODE,
			free_next: NO_NODE,
			height: 1,
		}
	}
}

/// A fixed-capacity AVL index of non-empty, half-open intervals keyed by `(start, end)`.
/// Intervals may overlap; inserting the same bounds replaces its value.
pub(crate) struct IntervalTree<T, const N: usize> {
	nodes: [IntervalNode<T>; N],
	root: u32,
	free_head: u32,
	len: usize,
}

impl<T, const N: usize> IntervalTree<T, N> {
	pub(crate) const fn new() -> Self {
		let mut nodes = [const { IntervalNode::empty() }; N];
		let mut index = 0;
		while index < N {
			nodes[index].free_next = if index + 1 < N {
				(index + 1) as u32
			} else {
				NO_NODE
			};
			index += 1;
		}
		Self {
			nodes,
			root: NO_NODE,
			free_head: if N == 0 { NO_NODE } else { 0 },
			len: 0,
		}
	}

	pub(crate) fn len(&self) -> usize {
		self.len
	}

	pub(crate) const fn capacity(&self) -> usize {
		N
	}

	pub(crate) fn is_empty(&self) -> bool {
		self.len == 0
	}

	pub(crate) fn is_full(&self) -> bool {
		self.len == N
	}

	/// Inserts an interval, returning its previous value if its bounds existed.
	/// Empty, reversed, or over-capacity inserts return the supplied value.
	pub(crate) fn insert(
		&mut self,
		range: Range<usize>,
		value: T,
	) -> Result<Option<T>, T> {
		if range.start >= range.end {
			return Err(value);
		}

		let (root, previous, inserted) = self.insert_node(self.root, range, value)?;
		self.root = root;
		if inserted {
			self.len += 1;
		}
		Ok(previous)
	}

	pub(crate) fn get(&self, range: &Range<usize>) -> Option<&T> {
		if range.start >= range.end {
			return None;
		}

		let mut current = self.root;
		while current != NO_NODE {
			let node = &self.nodes[current as usize];
			match Self::compare_ranges(range, &node.range) {
				Ordering::Less => current = node.left,
				Ordering::Greater => current = node.right,
				Ordering::Equal => return node.value.as_ref(),
			}
		}
		None
	}

	/// Returns the earliest indexed interval containing `point`.
	pub(crate) fn find_point(&self, point: usize) -> Option<(&Range<usize>, &T)> {
		let node_index = self.find_point_node(self.root, point)?;
		let node = &self.nodes[node_index as usize];
		Some((&node.range, node.value.as_ref()?))
	}

	/// Returns the earliest indexed interval overlapping `query`.
	pub(crate) fn find_overlap(&self, query: Range<usize>) -> Option<(&Range<usize>, &T)> {
		if query.start >= query.end {
			return None;
		}
		let node_index = self.find_overlap_node(self.root, &query)?;
		let node = &self.nodes[node_index as usize];
		Some((&node.range, node.value.as_ref()?))
	}

	/// Visits every interval overlapping `query` in ascending bound order.
	pub(crate) fn for_each_overlap(
		&self,
		query: Range<usize>,
		mut visit: impl FnMut(&Range<usize>, &T),
	) {
		if query.start < query.end {
			self.visit_overlap_nodes(self.root, &query, &mut visit);
		}
	}

	/// Finds the first aligned free subrange inside `bounds`.
	/// Indexed intervals are treated as occupied, including overlapping ones.
	pub(crate) fn first_fit(
		&self,
		bounds: Range<usize>,
		length: usize,
		alignment: usize,
	) -> Option<Range<usize>> {
		if bounds.start >= bounds.end
			|| length == 0
			|| alignment == 0
			|| !alignment.is_power_of_two()
		{
			return None;
		}

		let mut cursor = bounds.start;
		if let Some(fit) = self.first_fit_node(
			self.root,
			&bounds,
			length,
			alignment,
			&mut cursor,
		) {
			return Some(fit);
		}
		Self::fit_in_gap(cursor, bounds.end, length, alignment)
	}

	pub(crate) fn remove(&mut self, range: &Range<usize>) -> Option<T> {
		if range.start >= range.end {
			return None;
		}
		let (root, value) = self.remove_node(self.root, range)?;
		self.root = root;
		self.len -= 1;
		Some(value)
	}

	pub(crate) fn clear(&mut self) {
		self.root = NO_NODE;
		self.free_head = if N == 0 { NO_NODE } else { 0 };
		self.len = 0;
		let mut index = 0;
		while index < N {
			self.nodes[index] = IntervalNode::empty();
			self.nodes[index].free_next = if index + 1 < N {
				(index + 1) as u32
			} else {
				NO_NODE
			};
			index += 1;
		}
	}

	fn insert_node(
		&mut self,
		node_index: u32,
		range: Range<usize>,
		value: T,
	) -> Result<(u32, Option<T>, bool), T> {
		if node_index == NO_NODE {
			if self.free_head == NO_NODE {
				return Err(value);
			}
			let new_index = self.free_head;
			self.free_head = self.nodes[new_index as usize].free_next;
			self.nodes[new_index as usize] = IntervalNode::new(range, value);
			return Ok((new_index, None, true));
		}

		let ordering = Self::compare_ranges(&range, &self.nodes[node_index as usize].range);
		if ordering == Ordering::Equal {
			let previous = self.nodes[node_index as usize].value.replace(value);
			return Ok((node_index, previous, false));
		}

		let child_index = match ordering {
			Ordering::Less => self.nodes[node_index as usize].left,
			Ordering::Greater => self.nodes[node_index as usize].right,
			Ordering::Equal => unreachable!(),
		};
		let (new_child, previous, inserted) =
			self.insert_node(child_index, range, value)?;
		match ordering {
			Ordering::Less => self.nodes[node_index as usize].left = new_child,
			Ordering::Greater => self.nodes[node_index as usize].right = new_child,
			Ordering::Equal => unreachable!(),
		}
		Ok((self.rebalance_node(node_index), previous, inserted))
	}

	fn remove_node(&mut self, node_index: u32, range: &Range<usize>) -> Option<(u32, T)> {
		if node_index == NO_NODE {
			return None;
		}
		let ordering = Self::compare_ranges(range, &self.nodes[node_index as usize].range);
		match ordering {
			Ordering::Less => {
				let left = self.nodes[node_index as usize].left;
				let (new_left, value) = self.remove_node(left, range)?;
				self.nodes[node_index as usize].left = new_left;
				Some((self.rebalance_node(node_index), value))
			}
			Ordering::Greater => {
				let right = self.nodes[node_index as usize].right;
				let (new_right, value) = self.remove_node(right, range)?;
				self.nodes[node_index as usize].right = new_right;
				Some((self.rebalance_node(node_index), value))
			}
			Ordering::Equal => {
				let left = self.nodes[node_index as usize].left;
				let right = self.nodes[node_index as usize].right;
				let value = self.nodes[node_index as usize].value.take()?;
				if left == NO_NODE || right == NO_NODE {
					let replacement = if left == NO_NODE { right } else { left };
					self.free_node(node_index);
					return Some((replacement, value));
				}

				let (new_right, successor_index) = self.take_min(right);
			self.nodes[node_index as usize].range =
					self.nodes[successor_index as usize].range.clone();
				self.nodes[node_index as usize].value =
					self.nodes[successor_index as usize].value.take();
				self.nodes[node_index as usize].right = new_right;
				self.free_node(successor_index);
				Some((self.rebalance_node(node_index), value))
			}
		}
	}

	fn take_min(&mut self, node_index: u32) -> (u32, u32) {
		let left = self.nodes[node_index as usize].left;
		if left == NO_NODE {
			return (self.nodes[node_index as usize].right, node_index);
		}
		let (new_left, minimum) = self.take_min(left);
		self.nodes[node_index as usize].left = new_left;
		(self.rebalance_node(node_index), minimum)
	}

	fn free_node(&mut self, node_index: u32) {
		self.nodes[node_index as usize] = IntervalNode::empty();
		self.nodes[node_index as usize].free_next = self.free_head;
		self.free_head = node_index;
	}

	fn find_point_node(&self, node_index: u32, point: usize) -> Option<u32> {
		if node_index == NO_NODE {
			return None;
		}
		let node = &self.nodes[node_index as usize];
		if node.max_end <= point {
			return None;
		}
		let left = node.left;
		if left != NO_NODE && self.nodes[left as usize].max_end > point {
			if let Some(found) = self.find_point_node(left, point) {
				return Some(found);
			}
		}
		if node.range.start <= point && point < node.range.end {
			return Some(node_index);
		}
		if node.range.start > point {
			return None;
		}
		self.find_point_node(node.right, point)
	}

	fn find_overlap_node(&self, node_index: u32, query: &Range<usize>) -> Option<u32> {
		if node_index == NO_NODE {
			return None;
		}
		let node = &self.nodes[node_index as usize];
		if node.max_end <= query.start {
			return None;
		}
		let left = node.left;
		if left != NO_NODE && self.nodes[left as usize].max_end > query.start {
			if let Some(found) = self.find_overlap_node(left, query) {
				return Some(found);
			}
		}
		if node.range.start < query.end && query.start < node.range.end {
			return Some(node_index);
		}
		if node.range.start >= query.end {
			return None;
		}
		self.find_overlap_node(node.right, query)
	}

	fn visit_overlap_nodes(
		&self,
		node_index: u32,
		query: &Range<usize>,
		visit: &mut impl FnMut(&Range<usize>, &T),
	) {
		if node_index == NO_NODE {
			return;
		}
		let node = &self.nodes[node_index as usize];
		if node.max_end <= query.start {
			return;
		}
		let left = node.left;
		if left != NO_NODE && self.nodes[left as usize].max_end > query.start {
			self.visit_overlap_nodes(left, query, visit);
		}
		let right = node.right;
		if node.range.start < query.end && query.start < node.range.end {
			visit(&node.range, node.value.as_ref().expect("active interval has a value"));
		}
		if node.range.start < query.end {
			self.visit_overlap_nodes(right, query, visit);
		}
	}

	fn first_fit_node(
		&self,
		node_index: u32,
		bounds: &Range<usize>,
		length: usize,
		alignment: usize,
		cursor: &mut usize,
	) -> Option<Range<usize>> {
		if node_index == NO_NODE {
			return None;
		}
		let node = &self.nodes[node_index as usize];
		if node.max_end <= bounds.start {
			return None;
		}
		let left = node.left;
		if left != NO_NODE && self.nodes[left as usize].max_end > bounds.start {
			if let Some(fit) = self.first_fit_node(left, bounds, length, alignment, cursor) {
				return Some(fit);
			}
		}
		if node.range.start >= bounds.end {
			return None;
		}
		if node.range.end > bounds.start {
			let occupied_start = node.range.start.max(bounds.start);
			let occupied_end = node.range.end.min(bounds.end);
			if occupied_start > *cursor {
				if let Some(fit) = Self::fit_in_gap(*cursor, occupied_start, length, alignment) {
					return Some(fit);
				}
			}
			*cursor = (*cursor).max(occupied_end);
		}
		self.first_fit_node(node.right, bounds, length, alignment, cursor)
	}

	fn fit_in_gap(
		start: usize,
		end: usize,
		length: usize,
		alignment: usize,
	) -> Option<Range<usize>> {
		let aligned_start = start.checked_add(alignment - 1)? & !(alignment - 1);
		let allocation_end = aligned_start.checked_add(length)?;
		(allocation_end <= end).then_some(aligned_start..allocation_end)
	}

	fn compare_ranges(left: &Range<usize>, right: &Range<usize>) -> Ordering {
		left.start
			.cmp(&right.start)
			.then_with(|| left.end.cmp(&right.end))
	}

	fn refresh(&mut self, node_index: u32) {
		let node = &self.nodes[node_index as usize];
		let left = node.left;
		let right = node.right;
		let left_height = self.height(left);
		let right_height = self.height(right);
		let left_max_end = if left == NO_NODE {
			0
		} else {
			self.nodes[left as usize].max_end
		};
		let right_max_end = if right == NO_NODE {
			0
		} else {
			self.nodes[right as usize].max_end
		};
		let node = &mut self.nodes[node_index as usize];
		node.height = 1 + left_height.max(right_height);
		node.max_end = node.range.end.max(left_max_end).max(right_max_end);
	}

	fn height(&self, node_index: u32) -> u8 {
		if node_index == NO_NODE {
			0
		} else {
			self.nodes[node_index as usize].height
		}
	}

	fn balance_factor(&self, node_index: u32) -> i16 {
		let node = &self.nodes[node_index as usize];
		self.height(node.left) as i16 - self.height(node.right) as i16
	}

	fn rebalance_node(&mut self, node_index: u32) -> u32 {
		self.refresh(node_index);
		let balance = self.balance_factor(node_index);
		if balance > 1 {
			let left = self.nodes[node_index as usize].left;
			if self.balance_factor(left) < 0 {
				let new_left = self.rotate_left(left);
				self.nodes[node_index as usize].left = new_left;
			}
			self.rotate_right(node_index)
		} else if balance < -1 {
			let right = self.nodes[node_index as usize].right;
			if self.balance_factor(right) > 0 {
				let new_right = self.rotate_right(right);
				self.nodes[node_index as usize].right = new_right;
			}
			self.rotate_left(node_index)
		} else {
			node_index
		}
	}

	fn rotate_left(&mut self, node_index: u32) -> u32 {
		let pivot = self.nodes[node_index as usize].right;
		let middle = self.nodes[pivot as usize].left;
		self.nodes[node_index as usize].right = middle;
		self.refresh(node_index);
		self.nodes[pivot as usize].left = node_index;
		self.refresh(pivot);
		pivot
	}

	fn rotate_right(&mut self, node_index: u32) -> u32 {
		let pivot = self.nodes[node_index as usize].left;
		let middle = self.nodes[pivot as usize].right;
		self.nodes[node_index as usize].left = middle;
		self.refresh(node_index);
		self.nodes[pivot as usize].right = node_index;
		self.refresh(pivot);
		pivot
	}
}
