/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::driver::virtio::console::Console;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::signal::{SIGINT, SIGQUIT, SIGTSTP};

const CONTROL_CHARACTER_COUNT: usize = 32;
const COOKED_INPUT_CAPACITY: usize = 16 * 1024;
const CANONICAL_LINE_CAPACITY: usize = 4096;
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
    const fn default_console() -> Self {
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
pub(crate) fn reset_for_test() {
    *CONSOLE_TTY.lock() = ConsoleTtyState::new();
}