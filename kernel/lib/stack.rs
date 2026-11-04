#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Stack<T: Copy, const CAPACITY: usize> {
    len: u16,
    entries: [T; CAPACITY],
}

impl<T: Copy, const CAPACITY: usize> Stack<T, CAPACITY> {
    pub(crate) const fn new(empty_value: T) -> Self {
        assert!(CAPACITY > 0 && CAPACITY <= u16::MAX as usize);
        Self {
            len: 0,
            entries: [empty_value; CAPACITY],
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len as usize
    }

    pub(crate) const fn capacity(&self) -> usize {
        CAPACITY
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) const fn is_full(&self) -> bool {
        self.len as usize >= CAPACITY
    }

    pub(crate) fn peek(&self) -> Option<T> {
        let last = self.len.checked_sub(1)?;
        self.entries.get(usize::from(last)).copied()
    }

    pub(crate) fn push(&mut self, value: T) -> Result<(), T> {
        let index = usize::from(self.len);
        let Some(entry) = self.entries.get_mut(index) else {
            return Err(value);
        };
        *entry = value;
        self.len += 1;
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        let last = self.len.checked_sub(1)?;
        self.len = last;
        self.entries.get(usize::from(last)).copied()
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(T) -> bool) -> usize {
        let old_len = usize::from(self.len);
        let mut new_len = 0;
        for read_index in 0..old_len {
            let value = self.entries[read_index];
            if keep(value) {
                self.entries[new_len] = value;
                new_len += 1;
            }
        }
        self.len = new_len as u16;
        old_len - new_len
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}