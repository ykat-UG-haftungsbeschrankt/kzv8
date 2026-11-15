/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod error;
mod dispatch;

pub(crate) use dispatch::{dispatch, has_syscall_class};
#[allow(unused_imports)]
pub(crate) use dispatch::with_unix_class;