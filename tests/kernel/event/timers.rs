
#[cfg(test)]
mod tests {
    use super::TimerQueue;

    #[test]
    fn expires_timers_in_deadline_order() {
        let mut queue = TimerQueue::<3>::new();
        let late = queue.schedule(30).unwrap();
        let first = queue.schedule(10).unwrap();
        let middle = queue.schedule(20).unwrap();

        assert_eq!(queue.schedule(40), None);
        assert_eq!(queue.next_deadline(), Some(10));
        assert_eq!(queue.pop_expired(9), None);
        assert_eq!(queue.pop_expired(10), Some(first));
        assert_eq!(queue.pop_expired(20), Some(middle));
        assert_eq!(queue.pop_expired(29), None);
        assert_eq!(queue.pop_expired(30), Some(late));
        assert_eq!(queue.next_deadline(), None);
    }

    #[test]
    fn cancellation_reuses_slots_without_accepting_stale_ids() {
        let mut queue = TimerQueue::<2>::new();
        let cancelled = queue.schedule(10).unwrap();
        let retained = queue.schedule(20).unwrap();

        assert!(queue.cancel(cancelled));
        let replacement = queue.schedule(30).unwrap();
        assert_ne!(replacement, cancelled);
        assert!(!queue.cancel(cancelled));
        assert!(queue.cancel(retained));
        assert_eq!(queue.next_deadline(), Some(30));
        assert_eq!(queue.pop_expired(30), Some(replacement));
    }

    #[test]
    fn cancelling_heap_root_preserves_order() {
        let mut queue = TimerQueue::<4>::new();
        let first = queue.schedule(5).unwrap();
        let later = queue.schedule(15).unwrap();
        let middle = queue.schedule(10).unwrap();

        assert!(queue.cancel(first));
        assert_eq!(queue.next_deadline(), Some(10));
        assert_eq!(queue.pop_expired(10), Some(middle));
        assert_eq!(queue.pop_expired(15), Some(later));
    }
}
