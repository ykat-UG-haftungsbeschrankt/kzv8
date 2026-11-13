/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{SchedulingParameters, SporadicParameters, SporadicServer, SCHED_SPORADIC_POLICY};

    fn configuration(max_replenishments: u32) -> SchedulingParameters {
        SchedulingParameters {
            policy: SCHED_SPORADIC_POLICY,
            priority: 80,
            sporadic: Some(SporadicParameters {
                low_priority: 20,
                replenishment_period_ns: 100_000_000,
                initial_budget_ns: 10_000_000,
                max_replenishments,
            }),
        }
    }

    #[test]
    fn replenishment_is_rounded_up_to_scheduler_tick() {
        let mut server = SporadicServer::configured(configuration(4));
        server.charge(10_000_000, 10_000_001);

        assert_eq!(server.remaining_budget_ns, 0);
        assert_eq!(server.replenishment_count, 1);
        assert_eq!(
            server.replenishments.first().unwrap().deadline_ns,
            120_000_000
        );

        server.replenish_due(119_999_999);
        assert_eq!(server.remaining_budget_ns, 0);
        server.replenish_due(120_000_000);
        assert_eq!(server.remaining_budget_ns, 10_000_000);
    }

    #[test]
    fn replenishments_coalesce_at_configured_maximum() {
        let mut server = SporadicServer::configured(configuration(1));
        server.charge(1_000_000, 10_000_000);
        server.charge(1_000_000, 20_000_000);

        assert_eq!(server.replenishment_count, 1);
        let replenishment = server.replenishments.first().unwrap();
        assert_eq!(replenishment.deadline_ns, 120_000_000);
        assert_eq!(replenishment.budget_ns, 2_000_000);
        server.replenish_due(119_999_999);
        assert_eq!(server.remaining_budget_ns, 8_000_000);
        server.replenish_due(120_000_000);
        assert_eq!(server.remaining_budget_ns, 10_000_000);
    }
