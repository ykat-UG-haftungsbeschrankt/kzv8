/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    InvalidDevice,
    UnsupportedVersion,
    MissingVersionFeature,
    UnsupportedFeature,
    FeatureNegotiation,
    QueueUnavailable,
    InvalidQueueSize,
}
