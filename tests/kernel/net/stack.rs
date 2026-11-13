/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{
        Ipv4Address, Ipv4RouteTable, TunDevice, TunTapMode, INITIAL_DEFAULT_GATEWAY,
        INITIAL_LOCAL_ADDRESS, INITIAL_PREFIX_LENGTH,
    };

    #[test]
    fn route_lookup_prefers_the_longest_prefix() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        routes
            .add(
                Ipv4Address([10, 0, 2, 0]),
                24,
                Some(Ipv4Address([10, 0, 2, 1])),
            )
            .unwrap();
        routes
            .add(
                Ipv4Address([10, 0, 2, 128]),
                25,
                Some(Ipv4Address([10, 0, 2, 2])),
            )
            .unwrap();
        assert_eq!(
            routes.next_hop(Ipv4Address([10, 0, 2, 200])),
            Some(Ipv4Address([10, 0, 2, 2]))
        );
        assert_eq!(
            routes.next_hop(Ipv4Address([203, 0, 113, 1])),
            Some(INITIAL_DEFAULT_GATEWAY)
        );
    }

    #[test]
    fn deleted_default_route_stops_fallback_routing() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        routes.remove(Ipv4Address([0; 4]), 0).unwrap();
        assert_eq!(routes.next_hop(Ipv4Address([203, 0, 113, 1])), None);
    }

    #[test]
    fn directed_broadcast_lookup_uses_connected_prefix_routes() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        assert!(routes.is_broadcast_address(Ipv4Address([10, 0, 2, 255])));
        assert!(!routes.is_broadcast_address(Ipv4Address([10, 0, 2, 254])));
        routes.add(Ipv4Address([10, 0, 0, 0]), 8, None).unwrap();
        routes
            .add(
                Ipv4Address([10, 255, 0, 0]),
                16,
                Some(Ipv4Address([192, 0, 2, 1])),
            )
            .unwrap();
        assert!(!routes.is_broadcast_address(Ipv4Address([10, 255, 255, 255])));
    }

    #[test]
    fn directed_broadcast_retains_tunnel_route_when_device_is_down() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        let device = TunDevice::new().unwrap();
        device.configure(TunTapMode::Tun).unwrap();
        device.set_ipv4_address(Ipv4Address([198, 51, 100, 1]));
        device.set_ipv4_prefix_length(24);
        device.set_up(true);
        routes
            .add_tunnel(Ipv4Address([198, 51, 100, 0]), 24, device.clone())
            .unwrap();

        let broadcast = Ipv4Address([198, 51, 100, 255]);
        assert!(routes.is_broadcast_address(broadcast));
        assert!(routes.uses_tunnel(broadcast));
        assert!(routes.tunnel_device(broadcast).is_some());

        device.set_up(false);
        assert!(routes.is_broadcast_address(broadcast));
        assert!(routes.uses_tunnel(broadcast));
        assert!(routes.tunnel_device(broadcast).is_none());
    }
