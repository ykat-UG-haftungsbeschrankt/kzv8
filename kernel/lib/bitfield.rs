/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

/*
 * BitField packs fixed bits into u64 words:
 *
 * bit:     0 .. 63 | 64 .. 127 | ...
 * storage: word 0  | word 1    | ...
 */

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BitField<const WORDS: usize> {
    words: [u64; WORDS],
}

impl<const WORDS: usize> BitField<WORDS> {
    pub(crate) const fn new() -> Self {
        Self { words: [0; WORDS] }
    }

    #[allow(dead_code)]
    pub(crate) const fn bit_capacity(&self) -> usize {
        WORDS * u64::BITS as usize
    }

    pub(crate) fn get(&self, index: usize) -> Option<bool> {
        let word = self.words.get(index / u64::BITS as usize)?;
        Some(word & (1u64 << (index % u64::BITS as usize)) != 0)
    }

    pub(crate) fn set(&mut self, index: usize, value: bool) -> bool {
        let Some(word) = self.words.get_mut(index / u64::BITS as usize) else {
            return false;
        };
        let mask = 1u64 << (index % u64::BITS as usize);
        if value {
            *word |= mask;
        } else {
            *word &= !mask;
        }
        true
    }

    pub(crate) fn find_first_set(&self) -> Option<usize> {
        self.words.iter().enumerate().find_map(|(word_index, word)| {
            (*word != 0).then(|| {
                word_index * u64::BITS as usize + word.trailing_zeros() as usize
            })
        })
    }

    pub(crate) fn find_first_zero(&self) -> Option<usize> {
        self.words.iter().enumerate().find_map(|(word_index, word)| {
            (*word != u64::MAX).then(|| {
                word_index * u64::BITS as usize + (!word).trailing_zeros() as usize
            })
        })
    }

    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.words.fill(0);
    }
}