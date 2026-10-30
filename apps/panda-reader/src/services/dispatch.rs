use super::{command::Command, worker::WorkerState};

pub(super) fn handle(command: Command, workspace: String, state: &WorkerState) {
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
            let state = state.clone();
            super::worker::long_job(reply, move |_| {
                state.reload_plugins().map_err(|error| error.to_string())
            });
        }
        Command::SetPluginEnabled { id, enabled, reply } => {
            let state = state.clone();
            super::worker::long_job(reply, move |_| {
                state
                    .set_plugin_enabled(&id, enabled)
                    .map_err(|error| error.to_string())
            });
        }
        Command::ImportPlugin { source, reply } => {
            super::worker::import_plugin(source, reply, state);
        }
        Command::RemovePlugin { id, reply } => {
            let state = state.clone();
            super::worker::long_job(reply, move |_| {
                state.remove_plugin(&id).map_err(|error| error.to_string())
            });
        }
        Command::Snapshot {
            scope,
            search,
            limit,
            after,
            include_feeds,
            reply,
        } => super::articles::snapshot(
            workspace.clone(),
            scope,
            search,
            limit,
            after,
            include_feeds,
            reply,
            state,
        ),
        Command::Article {
            id,
            show_translation,
            translation_layout,
            hide_images,
            paragraph_indent,
            reply,
        } => super::articles::article(
            workspace.clone(),
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
        } => super::articles::save_reading_progress(workspace.clone(), id, progress, reply, state),
        Command::MarkAllRead {
            scope,
            search,
            reply,
        } => super::articles::mark_all_read(workspace.clone(), scope, search, reply, state),
        Command::Mark {
            id,
            field,
            value,
            reply,
        } => super::articles::mark(workspace.clone(), id, field, value, reply, state),
        Command::Extract {
            id,
            force,
            extractor,
            reply,
        } => super::articles::extract(workspace.clone(), id, force, extractor, reply, state),
        Command::Translate {
            id,
            target_lang,
            content_revision,
            translator_id,
            translation_layout,
            hide_images,
            paragraph_indent,
            progress,
            reply,
        } => super::articles::translate(
            workspace.clone(),
            id,
            target_lang,
            content_revision,
            translator_id,
            translation_layout,
            hide_images,
            paragraph_indent,
            reply,
            progress,
            state,
        ),
        Command::TranslateTitles {
            items,
            target_lang,
            force,
            reply,
        } => super::articles::translate_titles(
            workspace.clone(),
            items,
            target_lang,
            force,
            reply,
            state,
        ),
        Command::TranslationUsage { reply } => {
            super::articles::translation_usage(workspace.clone(), reply, state)
        }
        Command::AddFeed { url, reply } => {
            super::subscriptions::add_feed(workspace.clone(), url, reply, state)
        }
        Command::RemoveFeed { id, reply } => {
            super::subscriptions::remove_feed(workspace.clone(), id, reply, state)
        }
        Command::UpdateFeed {
            id,
            title,
            folder,
            feed_url,
            auto_translate_titles,
            reply,
        } => super::subscriptions::update_feed(
            workspace.clone(),
            id,
            title,
            folder,
            feed_url,
            auto_translate_titles,
            reply,
            state,
        ),
        Command::SetFeedAutoTranslateTitles { id, enabled, reply } => {
            super::subscriptions::set_feed_auto_translate_titles(
                workspace.clone(),
                id,
                enabled,
                reply,
                state,
            )
        }
        Command::ResetTitleTranslationCutoffs { reply } => {
            super::subscriptions::reset_title_translation_cutoffs(workspace.clone(), reply, state)
        }
        Command::ImportOpml { content, reply } => {
            super::subscriptions::import_opml(workspace.clone(), content, reply, state)
        }
        Command::ExportOpml { reply } => {
            super::subscriptions::export_opml(workspace.clone(), reply, state)
        }
        Command::ImportBackupFeeds { feeds, reply } => {
            let database = state.database.clone();
            super::worker::long_job(reply, move |_| {
                database.write("local".into(), move |store| {
                    store.merge_local_backup_feeds(&feeds)
                })
            });
        }
        Command::Connect {
            kind,
            endpoint,
            username,
            secret,
            reply,
        } => super::connections::connect(kind, endpoint, username, secret, reply, state),
        Command::Disconnect { kind, reply } => super::connections::disconnect(kind, reply, state),
        Command::RefreshFeed { id, reply } => {
            super::sync::refresh_feed(workspace.clone(), id, reply, state)
        }
        Command::Refresh { force, reply } => super::sync::refresh(workspace, force, reply, state),
    }
}
