use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use panda_core::{ArticleSummary, Scope};
use std::{sync::Arc, time::Duration};

pub(in crate::ui) struct ArticleList {
    _subscriptions: Vec<Subscription>,
    pub(in crate::ui) search_input: Entity<InputState>,
    pub(in crate::ui) articles: Arc<Vec<ArticleSummary>>,
    pub(in crate::ui) has_more: bool,
    pub(in crate::ui) scroll: UniformListScrollHandle,
    pub(in crate::ui) scope: Scope,
    pub(in crate::ui) search: String,
    pub(in crate::ui) search_revision: u64,
    pub(in crate::ui) snapshot_revision: u64,
    pub(in crate::ui) is_loading: bool,
    pub(in crate::ui) is_loading_more: bool,
}

impl ArticleList {
    pub(in crate::ui) fn new(
        window: &mut Window,
        cx: &mut Context<ReaderWindow>,
        language: crate::app::preferences::Language,
    ) -> Self {
        let search_placeholder = i18n::text(language, "Search title, author and body");
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder(search_placeholder));
        let search_for_events = search_input.clone();
        let _subscriptions =
            vec![
                cx.subscribe_in(&search_input, window, move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.list.search = search_for_events.read(cx).value().to_string();
                        this.list.search_revision = this.list.search_revision.wrapping_add(1);
                        let revision = this.list.search_revision;
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(200))
                                .await;
                            let _ = this.update(cx, |this, cx| {
                                if revision != this.list.search_revision {
                                    return;
                                }
                                this.load_snapshot(cx);
                            });
                        })
                        .detach();
                    }
                }),
            ];
        Self {
            search_input,
            articles: Arc::new(Vec::new()),
            has_more: false,
            scroll: UniformListScrollHandle::new(),
            scope: Scope::All,
            search: String::new(),
            search_revision: 0,
            snapshot_revision: 0,
            is_loading: true,
            is_loading_more: false,
            _subscriptions,
        }
    }
}
