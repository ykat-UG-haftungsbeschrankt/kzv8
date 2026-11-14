/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) fn reset_for_test() {
    *super::CONSOLE_TTY.lock() = super::ConsoleTtyState::new();
}