#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Completion {
    pub(crate) id: u64,
    pub(crate) status: i32,
}

pub(crate) struct CompletionQueue<T: Copy, const N: usize> {
    entries: [Option<T>; N],
    head: usize,
    length: usize,
}

impl<T: Copy, const N: usize> CompletionQueue<T, N> {
    pub(crate) const fn new() -> Self {
        assert!(N > 0);
        Self {
            entries: [None; N],
            head: 0,
            length: 0,
        }
    }

    pub(crate) fn push(&mut self, entry: T) -> Result<(), T> {
        if self.length == N {
            return Err(entry);
        }
        let index = (self.head + self.length) % N;
        self.entries[index] = Some(entry);
        self.length += 1;
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.length == 0 {
            return None;
        }
        let entry = self.entries[self.head].take();
        self.head = (self.head + 1) % N;
        self.length -= 1;
        entry
    }

    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(T) -> bool) {
        let mut entries = [None; N];
        let mut length = 0;
        while let Some(entry) = self.pop() {
            if keep(entry) {
                entries[length] = Some(entry);
                length += 1;
            }
        }
        self.entries = entries;
        self.head = 0;
        self.length = length;
    }
}
