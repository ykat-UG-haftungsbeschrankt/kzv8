/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

mod script;

#[allow(dead_code)]
pub(crate) mod beam;
#[allow(dead_code)]
pub(crate) mod ebpf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScriptFormat {
    Beam,
    Ebpf,
}

pub(crate) enum Script {
    Beam(beam::BeamModule),
    Ebpf(ebpf::Program),
}