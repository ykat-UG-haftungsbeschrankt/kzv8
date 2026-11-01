pub(super) fn initialize() {
    crate::arch::initialize_mmu();
}

pub(crate) const PAGE_SIZE: usize = 4096;

pub(crate) fn is_identity_mapped(address: usize) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        address < 0x1_0000_0000
    }
    #[cfg(target_arch = "aarch64")]
    {
        address < 0x4000_0000 || (0x4000_0000..0x8000_0000).contains(&address)
    }
}

pub(crate) fn translate(address: usize) -> Option<usize> {
    is_identity_mapped(address).then_some(address)
}
