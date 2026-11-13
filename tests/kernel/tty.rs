/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

pub(crate) fn reset_for_test() {
    *super::CONSOLE_TTY.lock() = super::ConsoleTtyState::new();
}

#[test]
fn pty_requires_unlock_and_applies_line_discipline() {
    let terminal = super::Pty::new().unwrap();
    assert_eq!(terminal.open_slave(), Err(Error::Acces));
    terminal.unlock_slave().unwrap();
    terminal.open_slave().unwrap();

    assert_eq!(terminal.master_write(b"hi\n"), Ok(3));
    let mut input = [0; 16];
    assert_eq!(terminal.slave_read(&mut input), Ok(3));
    assert_eq!(&input[..3], b"hi\n");

    let mut output = [0; 16];
    assert_eq!(terminal.master_read(&mut output), Ok(4));
    assert_eq!(&output[..4], b"hi\r\n");
    assert_eq!(terminal.slave_write(b"ok\n"), Ok(3));
    assert_eq!(terminal.master_read(&mut output), Ok(4));
    assert_eq!(&output[..4], b"ok\r\n");
}

#[test]
fn pty_tracks_session_foreground_group_and_window_size() {
    let terminal = super::Pty::new().unwrap();
    assert!(terminal.acquire_session(7, 7));
    assert!(terminal.session_owns(7));
    assert_eq!(terminal.foreground_process_group(7), Some(7));
    assert!(!terminal.acquire_session(8, 8));
    assert!(!terminal.set_foreground_process_group(8, 9));
    assert!(terminal.set_foreground_process_group(7, 9));
    assert_eq!(terminal.foreground_process_group(7), Some(9));

    let size = super::WindowSize {
        rows: 40,
        columns: 120,
        x_pixels: 0,
        y_pixels: 0,
    };
    terminal.set_window_size(size).unwrap();
    assert_eq!(terminal.window_size(), Ok(size));
}