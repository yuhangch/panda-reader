use super::{command::Command, worker::WorkerState};

pub(super) fn handle(command: Command, state: &WorkerState) {
    match command {
        Command::CommunityPluginCatalog { reply } => {
            super::worker::community_plugin_catalog(reply);
        }
        Command::InstallCommunityPlugin { id, reply } => {
            super::worker::install_community_plugin(id, reply, state);
        }
        Command::PluginList { reply } => {
            let _ = reply.send(state.plugin_list().map_err(|error| error.to_string()));
        }
        Command::ReloadPlugins { reply } => {
            let _ = reply.send(state.reload_plugins().map_err(|error| error.to_string()));
        }
        Command::SetPluginEnabled { id, enabled, reply } => {
            let _ = reply.send(
                state
                    .set_plugin_enabled(&id, enabled)
                    .map_err(|error| error.to_string()),
            );
        }
        Command::ImportPlugin { source, reply } => {
            super::worker::import_plugin(source, reply, state);
        }
        Command::RemovePlugin { id, reply } => {
            let _ = reply.send(state.remove_plugin(&id).map_err(|error| error.to_string()));
        }
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
            paragraph_indent,
            reply,
        } => super::articles::article(
            id,
            show_translation,
            translation_layout,
            hide_images,
            paragraph_indent,
            reply,
            state,
        ),
        Command::SaveReadingProgress {
            id,
            progress,
            reply,
        } => super::articles::save_reading_progress(id, progress, reply, state),
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
            content_revision,
            translator_id,
            translation_layout,
            hide_images,
            paragraph_indent,
            reply,
        } => super::articles::translate(
            id,
            target_lang,
            content_revision,
            translator_id,
            translation_layout,
            hide_images,
            paragraph_indent,
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
        Command::Refresh { force, reply } => super::sync::refresh(force, reply, state),
    }
}
