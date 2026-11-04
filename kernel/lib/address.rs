#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct PhysAddr(usize);

impl PhysAddr {
    pub(crate) const fn new(address: usize) -> Self {
        Self(address)
    }

    pub(crate) const fn as_usize(self) -> usize {
        self.0
    }

    pub(crate) fn checked_add(self, offset: usize) -> Option<Self> {
        self.0.checked_add(offset).map(Self)
    }

    pub(crate) fn checked_sub(self, base: Self) -> Option<usize> {
        self.0.checked_sub(base.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AddressTranslator {
    base: PhysAddr,
    length: usize,
}

impl AddressTranslator {
    pub(crate) fn new(base: PhysAddr, length: usize) -> Option<Self> {
        if length == 0 || base.checked_add(length).is_none() {
            return None;
        }
        Some(Self { base, length })
    }

    pub(crate) fn translate(&self, offset: usize) -> Option<PhysAddr> {
        if offset >= self.length {
            return None;
        }
        self.base.checked_add(offset)
    }

    pub(crate) fn offset_of(&self, address: PhysAddr) -> Option<usize> {
        let offset = address.checked_sub(self.base)?;
        (offset < self.length).then_some(offset)
    }
}