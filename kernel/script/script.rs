/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

use super::{beam, ebpf, Script, ScriptFormat};

impl Script {
    pub(crate) fn load(format: ScriptFormat, bytes: &[u8]) -> Result<Self, Error> {
        match format {
            ScriptFormat::Beam => Ok(Self::Beam(beam::BeamModule::load(bytes)?)),
            ScriptFormat::Ebpf => Ok(Self::Ebpf(ebpf::Program::load(bytes)?)),
        }
    }

    pub(crate) const fn format(&self) -> ScriptFormat {
        match self {
            Self::Beam(_) => ScriptFormat::Beam,
            Self::Ebpf(_) => ScriptFormat::Ebpf,
        }
    }
}