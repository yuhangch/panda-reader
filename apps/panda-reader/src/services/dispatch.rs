use super::{command::Command, worker::WorkerState};

pub(super) fn handle(command: Command, state: &WorkerState) {
    match command {
        Command::Snapshot {
            scope,
            search,
            limit,
            after,
            include_feeds,
            reply,
        } => super::articles::snapshot(scope, search, limit, after, include_feeds, reply, state),
        Command::Article {
            id,
            show_translation,
            translation_layout,
            hide_images,
            reply,
        } => super::articles::article(
            id,
            show_translation,
            translation_layout,
            hide_images,
            reply,
            state,
        ),
        Command::MarkAllRead { scope, reply } => {
            super::articles::mark_all_read(scope, reply, state)
        }
        Command::Mark {
            id,
            field,
            value,
            reply,
        } => super::articles::mark(id, field, value, reply, state),
        Command::Extract {
            id,
            force,
            extractor,
            reply,
        } => super::articles::extract(id, force, extractor, reply, state),
        Command::Translate {
            id,
            target_lang,
            translation_layout,
            hide_images,
            reply,
        } => super::articles::translate(
            id,
            target_lang,
            translation_layout,
            hide_images,
            reply,
            state,
        ),
        Command::TranslateTitles {
            items,
            target_lang,
            reply,
        } => super::articles::translate_titles(items, target_lang, reply, state),
        Command::TranslationUsage { reply } => super::articles::translation_usage(reply, state),
        Command::AddFeed { url, reply } => super::subscriptions::add_feed(url, reply, state),
        Command::RemoveFeed { id, reply } => super::subscriptions::remove_feed(id, reply, state),
        Command::UpdateFeed {
            id,
            title,
            folder,
            feed_url,
            auto_translate_titles,
            reply,
        } => super::subscriptions::update_feed(
            id,
            title,
            folder,
            feed_url,
            auto_translate_titles,
            reply,
            state,
        ),
        Command::SetFeedAutoTranslateTitles { id, enabled, reply } => {
            super::subscriptions::set_feed_auto_translate_titles(id, enabled, reply, state)
        }
        Command::ImportOpml { content, reply } => {
            super::subscriptions::import_opml(content, reply, state)
        }
        Command::ExportOpml { reply } => super::subscriptions::export_opml(reply, state),
        Command::Connect {
            kind,
            endpoint,
            username,
            secret,
            reply,
        } => super::connections::connect(kind, endpoint, username, secret, reply, state),
        Command::Disconnect { kind, reply } => super::connections::disconnect(kind, reply, state),
        Command::RefreshFeed { id, reply } => super::sync::refresh_feed(id, reply, state),
        Command::Refresh { reply } => super::sync::refresh(reply, state),
        Command::EnsureFavicon {
            host,
            site_url,
            icons_dir,
            reply,
        } => super::favicon::ensure_favicon(host, site_url, icons_dir, reply, state),
    }
}
