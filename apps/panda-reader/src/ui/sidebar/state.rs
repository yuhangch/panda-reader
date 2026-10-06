use gpui_kit::base::VirtualListScrollHandle;
use panda_core::Feed;
use std::{collections::HashSet, path::PathBuf};

pub(in crate::ui) struct Sidebar {
    pub(in crate::ui) feeds: Vec<Feed>,
    pub(in crate::ui) scroll: VirtualListScrollHandle,
    pub(in crate::ui) pending_remove_feed: Option<i64>,
    pub(in crate::ui) favicon_requested: HashSet<String>,
    pub(in crate::ui) icons_dir: PathBuf,
    pub(in crate::ui) is_refreshing: bool,
}

impl Sidebar {
    pub(in crate::ui) fn new(icons_dir: PathBuf) -> Self {
        Self {
            feeds: Vec::new(),
            scroll: VirtualListScrollHandle::new(),
            pending_remove_feed: None,
            favicon_requested: HashSet::new(),
            icons_dir,
            is_refreshing: false,
        }
    }
}
