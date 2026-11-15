/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;

use crate::kernel::error::Error;
use crate::kernel::vfs::{FileSystem, Node, VfsFuture, VfsResult};

pub(crate) const TIOCGWINSZ: u64 = 0x5413;
pub(crate) const TIOCSWINSZ: u64 = 0x5414;
pub(crate) const TIOCGPGRP: u64 = 0x540f;
pub(crate) const TIOCSPGRP: u64 = 0x5410;
pub(crate) const TCGETS: u64 = 0x5401;
pub(crate) const TCSETS: u64 = 0x5402;
pub(crate) const TCSETSW: u64 = 0x5403;
pub(crate) const TCSETSF: u64 = 0x5404;
pub(crate) const TCSBRK: u64 = 0x5409;
pub(crate) const TCXONC: u64 = 0x540a;
pub(crate) const TCFLSH: u64 = 0x540b;

const CONTROL_CHARACTER_COUNT: usize = 32;
pub(super) const CANONICAL_LINE_CAPACITY: usize = 4096;
const VINTR: usize = 0;
const VQUIT: usize = 1;
const VERASE: usize = 2;
const VKILL: usize = 3;
const VEOF: usize = 4;
const VTIME: usize = 5;
const VMIN: usize = 6;
const VSTART: usize = 8;
const VSTOP: usize = 9;
const VSUSP: usize = 10;
const VEOL: usize = 11;

const IGNCR: u32 = 0x0080;
const ICRNL: u32 = 0x0100;
const INLCR: u32 = 0x0040;
const ISTRIP: u32 = 0x0020;
const IXON: u32 = 0x0400;
const OPOST: u32 = 0x0001;
const ONLCR: u32 = 0x0004;
const OCRNL: u32 = 0x0008;
const ONOCR: u32 = 0x0010;
const ONLRET: u32 = 0x0020;
const CREAD: u32 = 0x0080;
const CLOCAL: u32 = 0x0800;
const CS8: u32 = 0x0030;
const ISIG: u32 = 0x0001;
const ICANON: u32 = 0x0002;
const ECHO: u32 = 0x0008;
const ECHOE: u32 = 0x0010;
const ECHOK: u32 = 0x0020;
const ECHONL: u32 = 0x0040;
const NOFLSH: u32 = 0x0080;
const TOSTOP: u32 = 0x0100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalAttributes {
    pub(crate) input_flags: u32,
    pub(crate) output_flags: u32,
    pub(crate) control_flags: u32,
    pub(crate) local_flags: u32,
    pub(crate) control_characters: [u8; CONTROL_CHARACTER_COUNT],
    pub(crate) input_speed: u32,
    pub(crate) output_speed: u32,
}

impl TerminalAttributes {
    pub(super) const fn default_console() -> Self {
        let mut control_characters = [0; CONTROL_CHARACTER_COUNT];
        control_characters[VINTR] = 3;
        control_characters[VQUIT] = 28;
        control_characters[VERASE] = 127;
        control_characters[VKILL] = 21;
        control_characters[VEOF] = 4;
        control_characters[VTIME] = 0;
        control_characters[VMIN] = 1;
        control_characters[VSTART] = 17;
        control_characters[VSTOP] = 19;
        control_characters[VSUSP] = 26;
        Self {
            input_flags: ICRNL | IXON,
            output_flags: OPOST | ONLCR,
            control_flags: CREAD | CLOCAL | CS8,
            local_flags: ISIG | ICANON | ECHO | ECHOE | ECHOK,
            control_characters,
            input_speed: 13,
            output_speed: 13,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WindowSize {
    pub(crate) rows: u16,
    pub(crate) columns: u16,
    pub(crate) x_pixels: u16,
    pub(crate) y_pixels: u16,
}

mod tty;

pub(crate) struct TtyFileSystem {
    terminal: Arc<Pty>,
}

impl TtyFileSystem {
    pub(crate) fn new_virtual_terminal() -> Result<Arc<Self>, Error> {
        let terminal = Pty::new()?;
        terminal.unlock_slave()?;
        Ok(Arc::new(Self { terminal }))
    }
}

impl FileSystem for TtyFileSystem {
    fn root(&self) -> VfsResult<Node> {
        tty::filesystem_root(Arc::clone(&self.terminal))
    }

    fn unmount(&self) -> VfsFuture<'_, VfsResult<()>> {
        self.terminal.close_master();
        Box::pin(async { Ok(()) })
    }
}

#[allow(unused_imports)]
pub(crate) use tty::{
    acquire_session, attributes, drain_output, flush_input, flush_output,
    foreground_process_group, flow, input_ready, ioctl_console, is_standard_terminal_descriptor, output_ready,
    read_console, session_owns_console, set_attributes, set_foreground_process_group,
    set_window_size, window_size, write_console,
};
#[allow(unused_imports)]
pub(crate) use tty::Pty;
#[allow(unused_imports)]
pub(crate) use tty::attach_initial_session;
#[cfg(test)]
pub(crate) use tty::tests;