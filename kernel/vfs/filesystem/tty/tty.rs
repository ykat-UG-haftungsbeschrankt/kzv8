/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::vfs::filesystem::virtio::console::Console;
use crate::kernel::error::Error;
use crate::kernel::credentials::{GroupId, UserId};
use crate::kernel::process::Credentials;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::{SIGINT, SIGQUIT, SIGTSTP};
use crate::kernel::vfs::{
    DirectoryEntry, DirectoryPlugin, FilePlugin, Node, NodeApi as _, NodeMetadata,
    NodeMetadataApi as _, OpenMode, VfsFuture, VfsResult,
};

use super::{
    CANONICAL_LINE_CAPACITY, ECHO, ECHOE, ECHOK, ECHONL, ICANON, ICRNL, IGNCR, INLCR, ISIG,
    ISTRIP, NOFLSH, OCRNL, ONLCR, ONLRET, ONOCR, OPOST, TOSTOP, VEOL, VEOF, VERASE, VINTR,
    VKILL, VMIN, VQUIT, VSUSP, VSTART, VSTOP, VTIME, IXON, TerminalAttributes, WindowSize,
};

const COOKED_INPUT_CAPACITY: usize = 16 * 1024;

mod pty;
pub(crate) use pty::Pty;

struct ConsoleTtyState {
    session_id: Option<i32>,
    foreground_process_group_id: Option<i32>,
    attributes: TerminalAttributes,
    cooked_input: [u8; COOKED_INPUT_CAPACITY],
    cooked_head: usize,
    cooked_length: usize,
    canonical_line: [u8; CANONICAL_LINE_CAPACITY],
    canonical_length: usize,
    eof_pending: bool,
    output_stopped: bool,
    output_column: usize,
    window_size: WindowSize,
}

impl ConsoleTtyState {
    const fn new() -> Self {
        Self {
            session_id: None,
            foreground_process_group_id: None,
            attributes: TerminalAttributes::default_console(),
            cooked_input: [0; COOKED_INPUT_CAPACITY],
            cooked_head: 0,
            cooked_length: 0,
            canonical_line: [0; CANONICAL_LINE_CAPACITY],
            canonical_length: 0,
            eof_pending: false,
            output_stopped: false,
            output_column: 0,
            window_size: WindowSize {
                rows: 25,
                columns: 80,
                x_pixels: 0,
                y_pixels: 0,
            },
        }
    }
}

static CONSOLE_TTY: IrqSpinLock<ConsoleTtyState> = IrqSpinLock::new(ConsoleTtyState::new());

pub(super) fn filesystem_root(terminal: Arc<Pty>) -> VfsResult<Node> {
    Ok(Node::directory(TtyDirectory { terminal }))
}

struct TtyDirectory {
    terminal: Arc<Pty>,
}

impl DirectoryPlugin for TtyDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        let result = match name {
            "console" | "slave" | "tty" => slave_node(Arc::clone(&self.terminal)),
            "master" => master_node(Arc::clone(&self.terminal)),
            _ => Err(Error::NoEnt),
        };
        Box::pin(async move { result })
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        let result = (|| {
            let mut entries = Vec::new();
            entries.try_reserve_exact(4).map_err(|_| Error::NoMem)?;
            entries.push(DirectoryEntry {
                name: String::from("master"),
                node: master_node(Arc::clone(&self.terminal))?,
            });
            entries.push(DirectoryEntry {
                name: String::from("slave"),
                node: slave_node(Arc::clone(&self.terminal))?,
            });
            entries.push(DirectoryEntry {
                name: String::from("console"),
                node: slave_node(Arc::clone(&self.terminal))?,
            });
            entries.push(DirectoryEntry {
                name: String::from("tty"),
                node: slave_node(Arc::clone(&self.terminal))?,
            });
            Ok(entries)
        })();
        Box::pin(async move { result })
    }
}

fn master_node(terminal: Arc<Pty>) -> VfsResult<Node> {
    let metadata = NodeMetadata::with_owner_and_mode(UserId::ROOT, GroupId::ROOT, 0o666)?;
    Ok(Node::file_with_metadata(PtyMasterFile { terminal }, metadata))
}

fn slave_node(terminal: Arc<Pty>) -> VfsResult<Node> {
    let metadata = NodeMetadata::with_owner_and_mode(UserId::ROOT, GroupId::ROOT, 0o666)?;
    Ok(Node::file_with_metadata(VirtualTerminalFile { terminal }, metadata))
}

struct VirtualTerminalFile {
    terminal: Arc<Pty>,
}

impl FilePlugin for VirtualTerminalFile {
    fn is_terminal(&self) -> bool {
        true
    }

    fn seekable(&self) -> bool {
        false
    }

    fn open(&self, _mode: OpenMode) -> VfsFuture<'_, VfsResult<Option<Arc<dyn FilePlugin>>>> {
        let opened_file = (|| {
            self.terminal.open_slave()?;
            if let Ok(process) = ThreadManager::current_process() {
                process.acquire_pty_terminal(Arc::clone(&self.terminal), false);
            }
            let file: Arc<dyn FilePlugin> = Arc::new(PtySlaveFile {
                terminal: Arc::clone(&self.terminal),
            });
            Ok(Some(file))
        })();
        Box::pin(async move { opened_file })
    }

    fn read_at<'a>(&'a self, _offset: u64, _buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async { Err(Error::NotTty) })
    }
}

struct PtySlaveFile {
    terminal: Arc<Pty>,
}

impl Drop for PtySlaveFile {
    fn drop(&mut self) {
        self.terminal.close_slave();
    }
}

impl FilePlugin for PtySlaveFile {
    fn is_terminal(&self) -> bool {
        true
    }

    fn seekable(&self) -> bool {
        false
    }

    fn read_at<'a>(&'a self, _offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.terminal.slave_read(buffer) })
    }

    fn write_at<'a>(&'a self, _offset: u64, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.terminal.slave_write(buffer) })
    }

    fn ioctl<'a>(
        &'a self,
        request: u64,
        argument: &'a mut [u8],
        _credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<i64>> {
        let result = pty_ioctl(&self.terminal, request, argument);
        Box::pin(async move { result })
    }
}

struct PtyMasterFile {
    terminal: Arc<Pty>,
}

impl FilePlugin for PtyMasterFile {
    fn is_terminal(&self) -> bool {
        true
    }

    fn seekable(&self) -> bool {
        false
    }

    fn read_at<'a>(&'a self, _offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.terminal.master_read(buffer) })
    }

    fn write_at<'a>(&'a self, _offset: u64, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.terminal.master_write(buffer) })
    }

    fn ioctl<'a>(
        &'a self,
        request: u64,
        argument: &'a mut [u8],
        _credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<i64>> {
        let result = pty_ioctl(&self.terminal, request, argument);
        Box::pin(async move { result })
    }
}

fn pty_ioctl(terminal: &Pty, request: u64, argument: &mut [u8]) -> VfsResult<i64> {
    match request {
        super::TIOCGWINSZ => {
            if argument.len() != 8 {
                return Err(Error::Inval);
            }
            let size = terminal.window_size()?;
            for (index, value) in [size.rows, size.columns, size.x_pixels, size.y_pixels]
                .into_iter()
                .enumerate()
            {
                let offset = index * 2;
                argument[offset..offset + 2].copy_from_slice(&value.to_ne_bytes());
            }
            Ok(0)
        }
        super::TIOCSWINSZ => {
            if argument.len() != 8 {
                return Err(Error::Inval);
            }
            let read_field = |index: usize| {
                let offset = index * 2;
                u16::from_ne_bytes([argument[offset], argument[offset + 1]])
            };
            terminal.set_window_size(WindowSize {
                rows: read_field(0),
                columns: read_field(1),
                x_pixels: read_field(2),
                y_pixels: read_field(3),
            })?;
            Ok(0)
        }
        super::TCGETS => {
            if argument.len() != 56 {
                return Err(Error::Inval);
            }
            let attributes = terminal.attributes()?;
            for (index, value) in [
                attributes.input_flags,
                attributes.output_flags,
                attributes.control_flags,
                attributes.local_flags,
            ]
            .into_iter()
            .enumerate()
            {
                write_native_u32(argument, index * 4, value);
            }
            argument[16..48].copy_from_slice(&attributes.control_characters);
            write_native_u32(argument, 48, attributes.input_speed);
            write_native_u32(argument, 52, attributes.output_speed);
            Ok(0)
        }
        super::TCSETS | super::TCSETSW | super::TCSETSF => {
            if argument.len() != 56 {
                return Err(Error::Inval);
            }
            let attributes = TerminalAttributes {
                input_flags: read_native_u32(argument, 0),
                output_flags: read_native_u32(argument, 4),
                control_flags: read_native_u32(argument, 8),
                local_flags: read_native_u32(argument, 12),
                control_characters: argument[16..48]
                    .try_into()
                    .map_err(|_| Error::Inval)?,
                input_speed: read_native_u32(argument, 48),
                output_speed: read_native_u32(argument, 52),
            };
            if request == super::TCSETSW {
                terminal.drain_output()?;
            } else if request == super::TCSETSF {
                terminal.flush_input();
            }
            terminal.set_attributes(attributes)?;
            Ok(0)
        }
        super::TCXONC => {
            if argument.len() != 4 {
                return Err(Error::Inval);
            }
            terminal.flow(read_native_i32(argument, 0))?;
            Ok(0)
        }
        super::TCFLSH => {
            if argument.len() != 4 {
                return Err(Error::Inval);
            }
            match read_native_i32(argument, 0) {
                0 => terminal.flush_input(),
                1 => terminal.flush_output(),
                2 => {
                    terminal.flush_input();
                    terminal.flush_output();
                }
                _ => return Err(Error::Inval),
            }
            Ok(0)
        }
        super::TCSBRK => {
            if argument.len() != 4 {
                return Err(Error::Inval);
            }
            terminal.drain_output()?;
            Ok(0)
        }
        _ => Err(Error::NoSys),
    }
}

fn read_native_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_native_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_ne_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn write_native_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

pub(crate) const fn is_standard_terminal_descriptor(file_descriptor: i32) -> bool {
    file_descriptor >= 0 && file_descriptor <= 2
}

pub(crate) fn input_ready() -> bool {
    let _ = poll_input();
    let terminal = CONSOLE_TTY.lock();
    terminal.cooked_length != 0 || terminal.eof_pending
}

pub(crate) fn output_ready() -> bool {
    !CONSOLE_TTY.lock().output_stopped && Console::is_writable()
}

fn poll_input() -> Result<(), Error> {
    if !Console::is_available() {
        return Ok(());
    }
    let mut raw = [0; 512];
    loop {
        let length = Console::read(&mut raw);
        if length == 0 {
            return Ok(());
        }
        for byte in &raw[..length] {
            let canonical = CONSOLE_TTY.lock().attributes.local_flags & ICANON != 0;
            let action = process_input_byte(*byte, canonical)?;
            if let Some(byte) = action.delivered {
                let mut terminal = CONSOLE_TTY.lock();
                if terminal.cooked_length == COOKED_INPUT_CAPACITY {
                    return Err(Error::NoBufs);
                }
                let tail = (terminal.cooked_head + terminal.cooked_length) % COOKED_INPUT_CAPACITY;
                terminal.cooked_input[tail] = byte;
                terminal.cooked_length += 1;
            }
            let flush_input = action.flush_input;
            apply_input_action(action)?;
            if flush_input {
                break;
            }
        }
    }
}

pub(crate) fn attach_initial_session(session_id: i32, process_group_id: i32) -> bool {
    let mut terminal = CONSOLE_TTY.lock();
    if terminal.session_id.is_some() {
        return false;
    }
    terminal.session_id = Some(session_id);
    terminal.foreground_process_group_id = Some(process_group_id);
    true
}

pub(crate) fn acquire_session(session_id: i32, process_group_id: i32) -> bool {
    let mut terminal = CONSOLE_TTY.lock();
    match terminal.session_id {
        Some(owner) => owner == session_id,
        None => {
            terminal.session_id = Some(session_id);
            terminal.foreground_process_group_id = Some(process_group_id);
            true
        }
    }
}

pub(crate) fn session_owns_console(session_id: i32) -> bool {
    CONSOLE_TTY.lock().session_id == Some(session_id)
}

pub(crate) fn foreground_process_group(session_id: i32) -> Option<i32> {
    let terminal = CONSOLE_TTY.lock();
    (terminal.session_id == Some(session_id))
        .then_some(terminal.foreground_process_group_id)
        .flatten()
}

pub(crate) fn set_foreground_process_group(session_id: i32, process_group_id: i32) -> bool {
    let mut terminal = CONSOLE_TTY.lock();
    if terminal.session_id != Some(session_id) {
        return false;
    }
    terminal.foreground_process_group_id = Some(process_group_id);
    true
}

pub(crate) fn attributes() -> TerminalAttributes {
    CONSOLE_TTY.lock().attributes
}

pub(crate) fn set_attributes(attributes: TerminalAttributes) -> Result<(), Error> {
    validate_attributes(attributes)?;
    CONSOLE_TTY.lock().attributes = attributes;
    Ok(())
}

pub(crate) fn validate_attributes(attributes: TerminalAttributes) -> Result<(), Error> {
    const INPUT_FLAGS: u32 = IGNCR | ICRNL | INLCR | ISTRIP | IXON;
    const OUTPUT_FLAGS: u32 = OPOST | ONLCR | OCRNL | ONOCR | ONLRET;
    const CONTROL_FLAGS: u32 = 0x0fff;
    const LOCAL_FLAGS: u32 = ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHONL | NOFLSH | 0x8000;
    if attributes.input_flags & !INPUT_FLAGS != 0
        || attributes.output_flags & !OUTPUT_FLAGS != 0
        || attributes.control_flags & !CONTROL_FLAGS != 0
        || attributes.local_flags & !LOCAL_FLAGS != 0
        || attributes.input_speed > 15
        || attributes.output_speed > 15
    {
        return Err(Error::Inval);
    }
    Ok(())
}

pub(crate) fn window_size() -> WindowSize {
    CONSOLE_TTY.lock().window_size
}

pub(crate) fn set_window_size(size: WindowSize) {
    CONSOLE_TTY.lock().window_size = size;
}

pub(crate) fn ioctl_console(request: u64, argument: &mut [u8]) -> Result<i64, Error> {
    match request {
        super::TIOCGWINSZ => {
            if argument.len() < 8 {
                return Err(Error::Inval);
            }
            let size = window_size();
            for (index, value) in [size.rows, size.columns, size.x_pixels, size.y_pixels]
                .into_iter()
                .enumerate()
            {
                let offset = index * 2;
                argument[offset..offset + 2].copy_from_slice(&value.to_ne_bytes());
            }
            Ok(0)
        }
        super::TIOCSWINSZ => {
            if argument.len() < 8 {
                return Err(Error::Inval);
            }
            let read_field = |index: usize| {
                let offset = index * 2;
                u16::from_ne_bytes([argument[offset], argument[offset + 1]])
            };
            set_window_size(WindowSize {
                rows: read_field(0),
                columns: read_field(1),
                x_pixels: read_field(2),
                y_pixels: read_field(3),
            });
            Ok(0)
        }
        super::TIOCGPGRP => {
            if argument.len() < 4 {
                return Err(Error::Inval);
            }
            let process = ThreadManager::current_process()?;
            let process_group = process.terminal_foreground_process_group()?;
            argument[..4].copy_from_slice(&process_group.to_ne_bytes());
            Ok(0)
        }
        super::TIOCSPGRP => {
            if argument.len() < 4 {
                return Err(Error::Inval);
            }
            let process_group = i32::from_ne_bytes(
                argument[..4].try_into().map_err(|_| Error::Inval)?,
            );
            ThreadManager::current_process()?.set_terminal_foreground_process_group(process_group)?;
            Ok(0)
        }
        super::TCGETS => {
            if argument.len() < 36 {
                return Err(Error::Inval);
            }
            let attributes = attributes();
            for (index, value) in [
                attributes.input_flags,
                attributes.output_flags,
                attributes.control_flags,
                attributes.local_flags,
            ]
            .into_iter()
            .enumerate()
            {
                write_native_u32(argument, index * 4, value);
            }
            argument[16] = 0;
            argument[17..36].copy_from_slice(&attributes.control_characters[..19]);
            Ok(0)
        }
        super::TCSETS | super::TCSETSW | super::TCSETSF => {
            if argument.len() < 36 {
                return Err(Error::Inval);
            }
            let mut attributes = attributes();
            attributes.input_flags = read_native_u32(argument, 0);
            attributes.output_flags = read_native_u32(argument, 4);
            attributes.control_flags = read_native_u32(argument, 8);
            attributes.local_flags = read_native_u32(argument, 12);
            attributes.control_characters[..19].copy_from_slice(&argument[17..36]);
            if request == super::TCSETSW {
                drain_output()?;
            } else if request == super::TCSETSF {
                flush_input();
            }
            set_attributes(attributes)?;
            Ok(0)
        }
        super::TCXONC => {
            if argument.len() < 4 {
                return Err(Error::Inval);
            }
            flow(read_native_i32(argument, 0))?;
            Ok(0)
        }
        super::TCFLSH => {
            if argument.len() < 4 {
                return Err(Error::Inval);
            }
            match read_native_i32(argument, 0) {
                0 => flush_input(),
                1 => flush_output(),
                2 => {
                    flush_input();
                    flush_output();
                }
                _ => return Err(Error::Inval),
            }
            Ok(0)
        }
        super::TCSBRK => {
            if argument.len() < 4 {
                return Err(Error::Inval);
            }
            drain_output()?;
            Ok(0)
        }
        _ => Err(Error::NoSys),
    }
}

pub(crate) fn flush_input() {
    {
        let mut terminal = CONSOLE_TTY.lock();
        terminal.cooked_head = 0;
        terminal.cooked_length = 0;
        terminal.canonical_length = 0;
        terminal.eof_pending = false;
    }
    Console::flush_input();
}

pub(crate) fn flush_output() {
    Console::flush_queued_output();
}

pub(crate) fn drain_output() -> Result<(), Error> {
    if !Console::is_available() {
        return Err(Error::Io);
    }
    while Console::output_pending() {
        if ThreadManager::current_signal_wake_pending() {
            return Err(Error::Intr);
        }
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
    Ok(())
}

pub(crate) fn flow(action: i32) -> Result<(), Error> {
    match action {
        0 => {
            CONSOLE_TTY.lock().output_stopped = true;
            Ok(())
        }
        1 => {
            CONSOLE_TTY.lock().output_stopped = false;
            Ok(())
        }
        2 | 3 => {
            let character = {
                let terminal = CONSOLE_TTY.lock();
                terminal.attributes.control_characters[if action == 2 { VSTOP } else { VSTART }]
            };
            if Console::write(&[character]) {
                Ok(())
            } else {
                Err(Error::Again)
            }
        }
        _ => Err(Error::Inval),
    }
}

pub(crate) fn write_console(input: &[u8], nonblocking: bool) -> Result<usize, Error> {
    if !Console::is_available() {
        return Err(Error::Io);
    }
    if CONSOLE_TTY.lock().attributes.local_flags & TOSTOP != 0 {
        if let Ok(process) = ThreadManager::current_process() {
            if let Ok(foreground) = process.terminal_foreground_process_group() {
                if foreground != process.process_group_id() {
                    let session_id = process.controlling_terminal_session()?;
                    crate::kernel::process::Process::signal_terminal_process_group(
                        session_id,
                        process.process_group_id(),
                        crate::kernel::signal::SIGTTOU,
                    )?;
                    return Err(Error::Intr);
                }
            }
        }
    }
    let mut written = 0;
    while written < input.len() {
        let (attributes, output_stopped, column) = {
            let terminal = CONSOLE_TTY.lock();
            (
                terminal.attributes,
                terminal.output_stopped,
                terminal.output_column,
            )
        };
        if output_stopped {
            if nonblocking {
                return if written == 0 { Err(Error::Again) } else { Ok(written) };
            }
            if ThreadManager::current_signal_wake_pending() {
                return if written == 0 { Err(Error::Intr) } else { Ok(written) };
            }
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
            continue;
        }

        let byte = input[written];
        let mut output = [0; 2];
        let output_length = if attributes.output_flags & OPOST == 0 {
            output[0] = byte;
            1
        } else if byte == b'\r' && attributes.output_flags & OCRNL != 0 {
            output[0] = b'\n';
            1
        } else if byte == b'\r' && attributes.output_flags & ONOCR != 0 && column == 0 {
            written += 1;
            continue;
        } else if byte == b'\n' && attributes.output_flags & ONLCR != 0 {
            output = [b'\r', b'\n'];
            2
        } else {
            output[0] = byte;
            1
        };

        if !Console::write(&output[..output_length]) {
            if nonblocking {
                return if written == 0 { Err(Error::Again) } else { Ok(written) };
            }
            if ThreadManager::current_signal_wake_pending() {
                return if written == 0 { Err(Error::Intr) } else { Ok(written) };
            }
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
            continue;
        }
        {
            let mut terminal = CONSOLE_TTY.lock();
            match byte {
                b'\r' => terminal.output_column = 0,
                b'\n' if attributes.output_flags & ONLRET != 0 => {
                    terminal.output_column = 0;
                }
                0x08 => terminal.output_column = terminal.output_column.saturating_sub(1),
                _ => terminal.output_column = terminal.output_column.saturating_add(1),
            }
        }
        written += 1;
    }
    Ok(written)
}

#[derive(Clone, Copy)]
struct InputAction {
    delivered: Option<u8>,
    echo: [u8; 3],
    echo_length: usize,
    signal: Option<u8>,
    flush_input: bool,
}

impl InputAction {
    const fn new() -> Self {
        Self {
            delivered: None,
            echo: [0; 3],
            echo_length: 0,
            signal: None,
            flush_input: false,
        }
    }
}

fn push_canonical_line(terminal: &mut ConsoleTtyState, ending: Option<u8>) -> bool {
    let extra = usize::from(ending.is_some());
    if terminal.canonical_length + extra > COOKED_INPUT_CAPACITY - terminal.cooked_length {
        return false;
    }
    for index in 0..terminal.canonical_length {
        let byte = terminal.canonical_line[index];
        let tail = (terminal.cooked_head + terminal.cooked_length) % COOKED_INPUT_CAPACITY;
        terminal.cooked_input[tail] = byte;
        terminal.cooked_length += 1;
    }
    if let Some(byte) = ending {
        let tail = (terminal.cooked_head + terminal.cooked_length) % COOKED_INPUT_CAPACITY;
        terminal.cooked_input[tail] = byte;
        terminal.cooked_length += 1;
    }
    true
}

fn read_cooked(
    terminal: &mut ConsoleTtyState,
    output: &mut [u8],
    canonical: bool,
) -> Option<usize> {
    if terminal.cooked_length == 0 {
        if terminal.eof_pending {
            terminal.eof_pending = false;
            return Some(0);
        }
        return None;
    }
    let mut length = 0;
    while length < output.len() && terminal.cooked_length != 0 {
        let byte = terminal.cooked_input[terminal.cooked_head];
        output[length] = byte;
        length += 1;
        terminal.cooked_head = (terminal.cooked_head + 1) % COOKED_INPUT_CAPACITY;
        terminal.cooked_length -= 1;
        if canonical
            && (byte == b'\n'
                || (terminal.attributes.control_characters[VEOL] != 0
                    && byte == terminal.attributes.control_characters[VEOL]))
        {
            break;
        }
    }
    Some(length)
}

fn process_input_byte(byte: u8, canonical: bool) -> Result<InputAction, Error> {
    let mut terminal = CONSOLE_TTY.lock();
    let attributes = terminal.attributes;
    let mut action = InputAction::new();
    let byte = if byte == b'\r' {
        if attributes.input_flags & IGNCR != 0 {
            return Ok(action);
        }
        if attributes.input_flags & ICRNL != 0 {
            b'\n'
        } else {
            byte
        }
    } else if byte == b'\n' && attributes.input_flags & INLCR != 0 {
        b'\r'
    } else {
        byte
    };
    let byte = if attributes.input_flags & ISTRIP != 0 {
        byte & 0x7f
    } else {
        byte
    };

    if attributes.input_flags & IXON != 0 {
        if byte == attributes.control_characters[VSTOP]
            && attributes.control_characters[VSTOP] != 0
        {
            terminal.output_stopped = true;
            return Ok(action);
        }
        if byte == attributes.control_characters[VSTART]
            && attributes.control_characters[VSTART] != 0
        {
            terminal.output_stopped = false;
            return Ok(action);
        }
    }

    if attributes.local_flags & ISIG != 0 {
        action.signal = if byte == attributes.control_characters[VINTR]
            && attributes.control_characters[VINTR] != 0
        {
            Some(SIGINT)
        } else if byte == attributes.control_characters[VQUIT]
            && attributes.control_characters[VQUIT] != 0
        {
            Some(SIGQUIT)
        } else if byte == attributes.control_characters[VSUSP]
            && attributes.control_characters[VSUSP] != 0
        {
            Some(SIGTSTP)
        } else {
            None
        };
        if action.signal.is_some() {
            if attributes.local_flags & NOFLSH == 0 {
                terminal.cooked_head = 0;
                terminal.cooked_length = 0;
                terminal.canonical_length = 0;
                terminal.eof_pending = false;
                action.flush_input = true;
            }
            if attributes.local_flags & ECHO != 0 {
                action.echo[0] = b'\n';
                action.echo_length = 1;
            }
            return Ok(action);
        }
    }

    if !canonical {
        action.delivered = Some(byte);
        if attributes.local_flags & ECHO != 0 {
            action.echo[0] = byte;
            action.echo_length = 1;
        }
        return Ok(action);
    }

    if byte == attributes.control_characters[VERASE] && attributes.control_characters[VERASE] != 0
    {
        if terminal.canonical_length != 0 {
            terminal.canonical_length -= 1;
            if attributes.local_flags & (ECHO | ECHOE) == (ECHO | ECHOE) {
                action.echo = [b'\x08', b' ', b'\x08'];
                action.echo_length = 3;
            }
        }
        return Ok(action);
    }
    if byte == attributes.control_characters[VKILL] && attributes.control_characters[VKILL] != 0
    {
        terminal.canonical_length = 0;
        if attributes.local_flags & (ECHO | ECHOK) == (ECHO | ECHOK) {
            action.echo[0] = b'\n';
            action.echo_length = 1;
        }
        return Ok(action);
    }
    if byte == attributes.control_characters[VEOF] && attributes.control_characters[VEOF] != 0 {
        if terminal.canonical_length == 0 {
            terminal.eof_pending = true;
        } else {
            if !push_canonical_line(&mut terminal, None) {
                return Err(Error::NoBufs);
            }
            terminal.canonical_length = 0;
        }
        return Ok(action);
    }

    let line_ending = byte == b'\n'
        || (attributes.control_characters[VEOL] != 0
            && byte == attributes.control_characters[VEOL]);
    if line_ending {
        if !push_canonical_line(&mut terminal, Some(byte)) {
            return Err(Error::NoBufs);
        }
        terminal.canonical_length = 0;
        if attributes.local_flags & ECHO != 0
            || (byte == b'\n' && attributes.local_flags & ECHONL != 0)
        {
            action.echo[0] = byte;
            action.echo_length = 1;
        }
        return Ok(action);
    }

    if terminal.canonical_length == CANONICAL_LINE_CAPACITY {
        if attributes.local_flags & ECHO != 0 {
            action.echo[0] = 7;
            action.echo_length = 1;
        }
        return Ok(action);
    }
    let canonical_length = terminal.canonical_length;
    terminal.canonical_line[canonical_length] = byte;
    terminal.canonical_length += 1;
    if attributes.local_flags & ECHO != 0 {
        action.echo[0] = byte;
        action.echo_length = 1;
    }
    Ok(action)
}

fn apply_input_action(action: InputAction) -> Result<(), Error> {
    if action.flush_input {
        Console::flush_input();
    }
    if let Some(signal) = action.signal {
        let (session, foreground) = {
            let terminal = CONSOLE_TTY.lock();
            (terminal.session_id, terminal.foreground_process_group_id)
        };
        if let (Some(session), Some(foreground)) = (session, foreground) {
            match crate::kernel::process::Process::signal_terminal_process_group(
                session,
                foreground,
                signal,
            ) {
                Ok(()) | Err(Error::Srch) => {}
                Err(error) => return Err(error),
            }
        }
    }
    if action.echo_length != 0 {
        let _ = write_console(&action.echo[..action.echo_length], false)?;
    }
    Ok(())
}

pub(crate) fn read_console(
    output: &mut [u8],
    nonblocking: bool,
) -> Result<usize, crate::kernel::error::Error> {
    if output.is_empty() {
        return Ok(0);
    }
    if !Console::is_available() {
        return Err(Error::Io);
    }
    check_background_read()?;
    loop {
        let attributes = CONSOLE_TTY.lock().attributes;
        let canonical = attributes.local_flags & ICANON != 0;
        let minimum = if canonical {
            1
        } else {
            usize::from(attributes.control_characters[VMIN]).min(output.len())
        };
        let timeout_ns = u64::from(attributes.control_characters[VTIME])
            .saturating_mul(100_000_000);
        let mut delivered = 0;
        let mut timeout_deadline = if !canonical && minimum == 0 && timeout_ns != 0 {
            Some(monotonic_time_ns().saturating_add(timeout_ns))
        } else {
            None
        };
        loop {
            poll_input()?;
            if delivered < output.len() {
                let mut terminal = CONSOLE_TTY.lock();
                let cooked_length = read_cooked(&mut terminal, &mut output[delivered..], canonical);
                drop(terminal);
                if let Some(length) = cooked_length {
                    if canonical {
                        if length == 0 {
                            ThreadManager::wait_if_process_stopped();
                            return Ok(0);
                        }
                        ThreadManager::wait_if_process_stopped();
                        return Ok(length);
                    }
                    delivered += length;
                    if delivered != 0 && minimum != 0 && timeout_ns != 0 {
                        timeout_deadline = Some(monotonic_time_ns().saturating_add(timeout_ns));
                    }
                }
            }

            if !canonical && minimum == 0 && delivered != 0 {
                ThreadManager::wait_if_process_stopped();
                return Ok(delivered);
            }
            if !canonical && minimum != 0 && delivered >= minimum {
                ThreadManager::wait_if_process_stopped();
                return Ok(delivered);
            }
            if !canonical && timeout_ns == 0 && minimum == 0 {
                return Ok(delivered);
            }
            if nonblocking {
                return if delivered == 0 {
                    Err(Error::Again)
                } else {
                    Ok(delivered)
                };
            }
            if let Some(deadline) = timeout_deadline {
                if monotonic_time_ns() >= deadline {
                    return Ok(delivered);
                }
            }
            if ThreadManager::current_signal_wake_pending() {
                return if delivered == 0 {
                    Err(Error::Intr)
                } else {
                    Ok(delivered)
                };
            }
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
        }
    }
}

fn check_background_read() -> Result<(), Error> {
    let process = ThreadManager::current_process()?;
    let Ok(foreground) = process.terminal_foreground_process_group() else {
        return Ok(());
    };
    if foreground == process.process_group_id() {
        return Ok(());
    }
    let session_id = process.controlling_terminal_session()?;
    crate::kernel::process::Process::signal_terminal_process_group(
        session_id,
        process.process_group_id(),
        crate::kernel::signal::SIGTTIN,
    )?;
    Err(Error::Intr)
}

fn monotonic_time_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/vfs/filesystem/tty/tty.rs"]
pub(crate) mod tests;