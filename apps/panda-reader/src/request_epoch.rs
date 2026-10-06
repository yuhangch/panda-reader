/// Invalidates asynchronous results whenever the reader's visible state changes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestEpoch(u64);

impl RequestEpoch {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }

    pub(crate) fn is_current(self, captured: u64) -> bool {
        self.0 == captured
    }
}

#[cfg(test)]
mod tests {
    use super::RequestEpoch;

    #[test]
    fn newer_reader_request_invalidates_older_responses() {
        let mut epoch = RequestEpoch::default();
        let old_article = epoch.next();
        let new_article = epoch.next();
        assert!(!epoch.is_current(old_article));
        assert!(epoch.is_current(new_article));

        let old_body_mode = epoch.next();
        let new_body_mode = epoch.next();
        assert!(!epoch.is_current(old_body_mode));
        assert!(epoch.is_current(new_body_mode));
    }
}
