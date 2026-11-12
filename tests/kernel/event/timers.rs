
/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;

    use super::TimerQueue;
    use crate::kernel::process::KernelObjectRef;

    fn timer_ref(timer: &Arc<super::Timer>) -> KernelObjectRef {
        KernelObjectRef::from_arc(timer)
    }

    #[test]
    fn expires_timers_in_deadline_order() {
        let mut queue = TimerQueue::<4>::new();
        let late = queue.schedule(30).unwrap();
        let first = queue.schedule(10).unwrap();
        let middle = queue.schedule(20).unwrap();
        let extra = queue.schedule(40).unwrap();
        assert_eq!(queue.schedule(50), None);
        let late_ref = timer_ref(&late);
        let first_ref = timer_ref(&first);
        let middle_ref = timer_ref(&middle);
        let extra_ref = timer_ref(&extra);

        assert_eq!(queue.next_deadline(), Some(10));
        assert_eq!(queue.pop_expired(9), None);
        assert_eq!(
            queue.pop_expired(10).map(|timer| timer_ref(&timer)),
            Some(first_ref)
        );
        assert_eq!(
            queue.pop_expired(20).map(|timer| timer_ref(&timer)),
            Some(middle_ref)
        );
        assert_eq!(queue.pop_expired(29), None);
        assert_eq!(
            queue.pop_expired(30).map(|timer| timer_ref(&timer)),
            Some(late_ref)
        );
        assert_eq!(
            queue.pop_expired(40).map(|timer| timer_ref(&timer)),
            Some(extra_ref)
        );
        assert_eq!(queue.next_deadline(), None);
    }

    #[test]
    fn timer_ids_are_heap_allocation_addresses() {
        let mut queue = TimerQueue::<1>::new();
        let timer_id = queue.schedule(10).unwrap();
        let timer_reference = timer_ref(&timer_id);
        let timer_in_tree = queue.deadlines.get(&(10, timer_reference.raw())).unwrap();
        assert!(Arc::ptr_eq(timer_in_tree, &timer_id));
    }

    #[test]
    fn address_only_stale_ids_can_alias_reused_allocations() {
        let mut queue = TimerQueue::<1>::new();
        let cancelled = queue.schedule(10).unwrap();
        let cancelled_ref = timer_ref(&cancelled);

        assert!(queue.cancel(&cancelled));
        drop(cancelled);
        assert!(queue.timer_by_address(cancelled_ref.raw()).is_none());
        let replacement = queue.schedule(30).unwrap();
        let replacement_ref = timer_ref(&replacement);
        let stale_timer = queue.timer_by_address(cancelled_ref.raw());
        assert_eq!(stale_timer.is_some(), replacement_ref == cancelled_ref);
        if let Some(stale_timer) = stale_timer {
            assert!(queue.cancel(&stale_timer));
        } else {
            assert!(queue.cancel(&replacement));
        }
        assert_eq!(queue.next_deadline(), None);
        assert!(queue.pop_expired(30).is_none());
    }

    #[test]
    fn cancelling_heap_root_preserves_order() {
        let mut queue = TimerQueue::<4>::new();
        let first = queue.schedule(5).unwrap();
        let later = queue.schedule(15).unwrap();
        let middle = queue.schedule(10).unwrap();
        let later_ref = timer_ref(&later);
        let middle_ref = timer_ref(&middle);

        assert!(queue.cancel(&first));
        assert_eq!(queue.next_deadline(), Some(10));
        assert_eq!(
            queue.pop_expired(10).map(|timer| timer_ref(&timer)),
            Some(middle_ref)
        );
        assert_eq!(
            queue.pop_expired(15).map(|timer| timer_ref(&timer)),
            Some(later_ref)
        );
    }
}
