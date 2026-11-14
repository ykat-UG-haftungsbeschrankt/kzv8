/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod block;
pub(crate) mod tty;
pub(crate) mod virtio;
#[cfg(feature = "virtualization")]
pub(crate) mod vm;
#[cfg(test)]
pub(crate) mod test;