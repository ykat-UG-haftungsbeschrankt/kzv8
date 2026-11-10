/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[inline(never)]
pub(crate) fn template_sizes(
    start: usize,
    data_end: usize,
    end: usize,
    capacity: usize,
) -> Option<(usize, usize)> {
    let template_size = end.checked_sub(start)?;
    let initialized_size = data_end.checked_sub(start)?;
    (initialized_size <= template_size && template_size <= capacity)
        .then_some((template_size, initialized_size))
}