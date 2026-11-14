/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::signal::{SIGINT, SIGQUIT, SIGTSTP, SIGWINCH};

const PTY_BUFFER_CAPACITY: usize = 16 * 1024;
const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;

struct ByteQueue {
    //@FIXME: INDEXTYPE(bytes); ring-buffer backing array; Vec provides O(1) indexed access, keep Vec.
    bytes: Vec<u8>,
    head: usize,
    length: usize,
}

impl ByteQueue {
    fn new(capacity: usize) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| Error::NoMem)?;
        bytes.resize(capacity, 0);
        Ok(Self {
            bytes,
            head: 0,
            length: 0,
        })
    }

    fn available(&self) -> usize {
        self.bytes.len() - self.length
    }

    fn is_empty(&self) -> bool {
        self.length == 0
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.length == self.bytes.len() {
            return false;
        }
        let tail = (self.head + self.length) % self.bytes.len();
        self.bytes[tail] = byte;
        self.length += 1;
        true
    }

    fn push_slice(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.available() {
            return false;
        }
        for byte in bytes {
            let _ = self.push(*byte);
        }
        true
    }

    fn pop_into(&mut self, output: &mut [u8]) -> usize {
        let count = output.len().min(self.length);
        for byte in &mut output[..count] {
            *byte = self.bytes[self.head];
            self.head = (self.head + 1) % self.bytes.len();
        }
        self.length -= count;
        if self.length == 0 {
            self.head = 0;
        }
        count
    }

    fn clear(&mut self) {
        self.head = 0;
        self.length = 0;
    }
}

struct PtyState {
    master_open: bool,
    slave_locked: bool,
    slave_open_count: usize,
    session_id: Option<i32>,
    foreground_process_group_id: Option<i32>,
    attributes: super::TerminalAttributes,
    window_size: super::WindowSize,
    to_slave: ByteQueue,
    to_master: ByteQueue,
    canonical_line: [u8; super::CANONICAL_LINE_CAPACITY],
    canonical_length: usize,
    eof_pending: bool,
    output_stopped: bool,
    output_column: usize,
}

impl PtyState {
    fn new(to_slave: ByteQueue, to_master: ByteQueue) -> Self {
        Self {
            master_open: true,
            slave_locked: true,
            slave_open_count: 0,
            session_id: None,
            foreground_process_group_id: None,
            attributes: super::TerminalAttributes::default_console(),
            window_size: super::WindowSize {
                rows: 25,
                columns: 80,
                x_pixels: 0,
                y_pixels: 0,
            },
            to_slave,
            to_master,
            canonical_line: [0; super::CANONICAL_LINE_CAPACITY],
            canonical_length: 0,
            eof_pending: false,
            output_stopped: false,
            output_column: 0,
        }
    }
}

pub(crate) struct Pty {
    state: IrqSpinLock<PtyState>,
}

#[derive(Clone, Copy)]
struct InputAction {
    accepted: bool,
    signal: Option<(i32, i32, u8)>,
}

impl InputAction {
    const fn accepted(signal: Option<(i32, i32, u8)>) -> Self {
        Self {
            accepted: true,
            signal,
        }
    }

    const fn rejected() -> Self {
        Self {
            accepted: false,
            signal: None,
        }
    }
}

impl Pty {
    pub(crate) fn new() -> Result<Arc<Self>, Error> {
        let to_slave = ByteQueue::new(PTY_BUFFER_CAPACITY)?;
        let to_master = ByteQueue::new(PTY_BUFFER_CAPACITY)?;
        Ok(Arc::new(Self {
            state: IrqSpinLock::new(PtyState::new(to_slave, to_master)),
        }))
    }

    pub(crate) fn unlock_slave(&self) -> Result<(), Error> {
        let mut state = self.state.lock();
        if !state.master_open {
            return Err(Error::Badf);
        }
        state.slave_locked = false;
        Ok(())
    }

    pub(crate) fn open_slave(&self) -> Result<(), Error> {
        let mut state = self.state.lock();
        if state.slave_locked {
            return Err(Error::Acces);
        }
        if !state.master_open {
            return Err(Error::Io);
        }
        state.slave_open_count = state
            .slave_open_count
            .checked_add(1)
            .ok_or(Error::Mfile)?;
        Ok(())
    }

    pub(crate) fn close_master(&self) {
        self.state.lock().master_open = false;
    }

    pub(crate) fn close_slave(&self) {
        let mut state = self.state.lock();
        state.slave_open_count = state.slave_open_count.saturating_sub(1);
    }

    pub(crate) fn master_read(&self, output: &mut [u8]) -> Result<usize, Error> {
        if output.is_empty() {
            return Ok(0);
        }
        let mut state = self.state.lock();
        let count = state.to_master.pop_into(output);
        if count != 0 {
            return Ok(count);
        }
        if !state.master_open || state.slave_open_count == 0 {
            Err(Error::Io)
        } else {
            Err(Error::Again)
        }
    }

    pub(crate) fn master_write(&self, input: &[u8]) -> Result<usize, Error> {
        let mut written = 0;
        for byte in input {
            let action = {
                let mut state = self.state.lock();
                if !state.master_open || state.slave_open_count == 0 {
                    return if written == 0 {
                        Err(Error::Io)
                    } else {
                        Ok(written)
                    };
                }
                process_master_input_byte(&mut state, *byte)
            };
            if !action.accepted {
                return if written == 0 {
                    Err(Error::Again)
                } else {
                    Ok(written)
                };
            }
            written += 1;
            if let Some((session_id, process_group_id, signal)) = action.signal {
                if let Err(error) = crate::kernel::process::Process::signal_terminal_process_group(
                    session_id,
                    process_group_id,
                    signal,
                ) {
                    if error != Error::Srch {
                        return Ok(written);
                    }
                }
            }
        }
        Ok(written)
    }

    pub(crate) fn slave_read(&self, output: &mut [u8]) -> Result<usize, Error> {
        if output.is_empty() {
            return Ok(0);
        }
        let mut state = self.state.lock();
        let count = state.to_slave.pop_into(output);
        if count != 0 {
            return Ok(count);
        }
        if state.eof_pending {
            state.eof_pending = false;
            return Ok(0);
        }
        if !state.master_open {
            Err(Error::Io)
        } else {
            Err(Error::Again)
        }
    }

    pub(crate) fn slave_write(&self, input: &[u8]) -> Result<usize, Error> {
        if input.is_empty() {
            return Ok(0);
        }
        let mut state = self.state.lock();
        if !state.master_open {
            return Err(Error::Io);
        }
        let mut written = 0;
        for byte in input {
            if !queue_output_byte(&mut state, *byte) {
                break;
            }
            written += 1;
        }
        if written == 0 {
            Err(Error::Again)
        } else {
            Ok(written)
        }
    }

    pub(crate) fn master_readiness(&self, events: i16) -> Result<i16, Error> {
        let state = self.state.lock();
        let mut readiness = 0;
        if events & POLLIN != 0
            && (!state.to_master.is_empty() || !state.master_open || state.slave_open_count == 0)
        {
            readiness |= POLLIN;
        }
        if events & POLLOUT != 0
            && state.master_open
            && state.slave_open_count != 0
            && state.to_slave.available() != 0
        {
            readiness |= POLLOUT;
        }
        if !state.master_open || state.slave_open_count == 0 {
            readiness |= POLLHUP;
        }
        if !state.master_open {
            readiness |= POLLERR;
        }
        Ok(readiness)
    }

    pub(crate) fn slave_readiness(&self, events: i16) -> Result<i16, Error> {
        let state = self.state.lock();
        let mut readiness = 0;
        if events & POLLIN != 0
            && (!state.to_slave.is_empty() || state.eof_pending || !state.master_open)
        {
            readiness |= POLLIN;
        }
        if events & POLLOUT != 0
            && state.master_open
            && !state.output_stopped
            && state.to_master.available() != 0
        {
            readiness |= POLLOUT;
        }
        if !state.master_open {
            readiness |= POLLHUP | POLLERR;
        }
        Ok(readiness)
    }

    pub(crate) fn attributes(&self) -> Result<super::TerminalAttributes, Error> {
        Ok(self.state.lock().attributes)
    }

    pub(crate) fn set_attributes(
        &self,
        attributes: super::TerminalAttributes,
    ) -> Result<(), Error> {
        super::validate_attributes(attributes)?;
        self.state.lock().attributes = attributes;
        Ok(())
    }

    pub(crate) fn window_size(&self) -> Result<super::WindowSize, Error> {
        Ok(self.state.lock().window_size)
    }

    pub(crate) fn set_window_size(&self, size: super::WindowSize) -> Result<(), Error> {
        let recipient = {
            let mut state = self.state.lock();
            if state.window_size == size {
                None
            } else {
                state.window_size = size;
                state
                    .session_id
                    .zip(state.foreground_process_group_id)
            }
        };
        if let Some((session_id, process_group_id)) = recipient {
            match crate::kernel::process::Process::signal_terminal_process_group(
                session_id,
                process_group_id,
                SIGWINCH,
            ) {
                Ok(()) | Err(Error::Srch) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn flush_input(&self) {
        let mut state = self.state.lock();
        state.to_slave.clear();
        state.canonical_length = 0;
        state.eof_pending = false;
    }

    pub(crate) fn flush_output(&self) {
        self.state.lock().to_master.clear();
    }

    pub(crate) fn flow(&self, action: i32) -> Result<(), Error> {
        let mut state = self.state.lock();
        match action {
            0 => state.output_stopped = true,
            1 => state.output_stopped = false,
            2 | 3 => {
                let index = if action == 2 { super::VSTOP } else { super::VSTART };
                let control = state.attributes.control_characters[index];
                if control != 0 && !state.to_slave.push(control) {
                    return Err(Error::Again);
                }
            }
            _ => return Err(Error::Inval),
        }
        Ok(())
    }

    pub(crate) fn acquire_session(&self, session_id: i32, process_group_id: i32) -> bool {
        let mut state = self.state.lock();
        match state.session_id {
            Some(owner) => owner == session_id,
            None => {
                state.session_id = Some(session_id);
                state.foreground_process_group_id = Some(process_group_id);
                true
            }
        }
    }

    pub(crate) fn session_owns(&self, session_id: i32) -> bool {
        self.state.lock().session_id == Some(session_id)
    }

    pub(crate) fn foreground_process_group(&self, session_id: i32) -> Option<i32> {
        let state = self.state.lock();
        (state.session_id == Some(session_id))
            .then_some(state.foreground_process_group_id)
            .flatten()
    }

    pub(crate) fn set_foreground_process_group(
        &self,
        session_id: i32,
        process_group_id: i32,
    ) -> bool {
        let mut state = self.state.lock();
        if state.session_id != Some(session_id) {
            return false;
        }
        state.foreground_process_group_id = Some(process_group_id);
        true
    }
}

fn process_master_input_byte(state: &mut PtyState, input: u8) -> InputAction {
    let attributes = state.attributes;
    let byte = if input == b'\r' {
        if attributes.input_flags & super::IGNCR != 0 {
            return InputAction::accepted(None);
        }
        if attributes.input_flags & super::ICRNL != 0 {
            b'\n'
        } else {
            input
        }
    } else if input == b'\n' && attributes.input_flags & super::INLCR != 0 {
        b'\r'
    } else {
        input
    };
    let byte = if attributes.input_flags & super::ISTRIP != 0 {
        byte & 0x7f
    } else {
        byte
    };

    if attributes.input_flags & super::IXON != 0 {
        if byte == attributes.control_characters[super::VSTOP]
            && attributes.control_characters[super::VSTOP] != 0
        {
            state.output_stopped = true;
            return InputAction::accepted(None);
        }
        if byte == attributes.control_characters[super::VSTART]
            && attributes.control_characters[super::VSTART] != 0
        {
            state.output_stopped = false;
            return InputAction::accepted(None);
        }
    }

    let signal_number = if attributes.local_flags & super::ISIG == 0 {
        None
    } else if byte == attributes.control_characters[super::VINTR]
        && attributes.control_characters[super::VINTR] != 0
    {
        Some(SIGINT)
    } else if byte == attributes.control_characters[super::VQUIT]
        && attributes.control_characters[super::VQUIT] != 0
    {
        Some(SIGQUIT)
    } else if byte == attributes.control_characters[super::VSUSP]
        && attributes.control_characters[super::VSUSP] != 0
    {
        Some(SIGTSTP)
    } else {
        None
    };
    if let Some(signal_number) = signal_number {
        if attributes.local_flags & super::NOFLSH == 0 {
            state.to_slave.clear();
            state.canonical_length = 0;
            state.eof_pending = false;
        }
        if attributes.local_flags & super::ECHO != 0 {
            echo_bytes(state, b"\n");
        }
        let signal = state
            .session_id
            .zip(state.foreground_process_group_id)
            .map(|(session_id, process_group_id)| {
                (session_id, process_group_id, signal_number)
            });
        return InputAction::accepted(signal);
    }

    if attributes.local_flags & super::ICANON == 0 {
        if !state.to_slave.push(byte) {
            return InputAction::rejected();
        }
        if attributes.local_flags & super::ECHO != 0 {
            echo_bytes(state, &[byte]);
        }
        return InputAction::accepted(None);
    }

    if byte == attributes.control_characters[super::VERASE]
        && attributes.control_characters[super::VERASE] != 0
    {
        if state.canonical_length != 0 {
            state.canonical_length -= 1;
            if attributes.local_flags & (super::ECHO | super::ECHOE)
                == (super::ECHO | super::ECHOE)
            {
                echo_bytes(state, b"\x08 \x08");
            }
        }
        return InputAction::accepted(None);
    }
    if byte == attributes.control_characters[super::VKILL]
        && attributes.control_characters[super::VKILL] != 0
    {
        state.canonical_length = 0;
        if attributes.local_flags & (super::ECHO | super::ECHOK)
            == (super::ECHO | super::ECHOK)
        {
            echo_bytes(state, b"\n");
        }
        return InputAction::accepted(None);
    }
    if byte == attributes.control_characters[super::VEOF]
        && attributes.control_characters[super::VEOF] != 0
    {
        if state.canonical_length == 0 {
            state.eof_pending = true;
        } else if !push_canonical_line(state, None) {
            return InputAction::rejected();
        }
        return InputAction::accepted(None);
    }

    let line_ending = byte == b'\n'
        || (attributes.control_characters[super::VEOL] != 0
            && byte == attributes.control_characters[super::VEOL]);
    if line_ending {
        if !push_canonical_line(state, Some(byte)) {
            return InputAction::rejected();
        }
        if attributes.local_flags & super::ECHO != 0
            || (byte == b'\n' && attributes.local_flags & super::ECHONL != 0)
        {
            echo_bytes(state, &[byte]);
        }
        return InputAction::accepted(None);
    }

    if state.canonical_length == state.canonical_line.len() {
        if attributes.local_flags & super::ECHO != 0 {
            echo_bytes(state, &[7]);
        }
        return InputAction::accepted(None);
    }
    let canonical_length = state.canonical_length;
    state.canonical_line[canonical_length] = byte;
    state.canonical_length += 1;
    if attributes.local_flags & super::ECHO != 0 {
        echo_bytes(state, &[byte]);
    }
    InputAction::accepted(None)
}

fn push_canonical_line(state: &mut PtyState, ending: Option<u8>) -> bool {
    let required = state.canonical_length + usize::from(ending.is_some());
    if required > state.to_slave.available() {
        return false;
    }
    if !state
        .to_slave
        .push_slice(&state.canonical_line[..state.canonical_length])
    {
        return false;
    }
    if let Some(byte) = ending {
        let _ = state.to_slave.push(byte);
    }
    state.canonical_length = 0;
    true
}

fn echo_bytes(state: &mut PtyState, bytes: &[u8]) {
    if state.output_stopped {
        return;
    }
    for byte in bytes {
        if !queue_output_byte(state, *byte) {
            break;
        }
    }
}

fn queue_output_byte(state: &mut PtyState, byte: u8) -> bool {
    if state.output_stopped {
        return false;
    }
    let mut output = [0; 2];
    let length = if state.attributes.output_flags & super::OPOST == 0 {
        output[0] = byte;
        1
    } else if byte == b'\r' && state.attributes.output_flags & super::OCRNL != 0 {
        output[0] = b'\n';
        1
    } else if byte == b'\r'
        && state.attributes.output_flags & super::ONOCR != 0
        && state.output_column == 0
    {
        0
    } else if byte == b'\n' && state.attributes.output_flags & super::ONLCR != 0 {
        output = [b'\r', b'\n'];
        2
    } else {
        output[0] = byte;
        1
    };
    if !state.to_master.push_slice(&output[..length]) {
        return false;
    }
    match byte {
        b'\r' => state.output_column = 0,
        b'\n' if state.attributes.output_flags & super::ONLRET != 0 => {
            state.output_column = 0;
        }
        0x08 => state.output_column = state.output_column.saturating_sub(1),
        _ => state.output_column = state.output_column.saturating_add(1),
    }
    true
}