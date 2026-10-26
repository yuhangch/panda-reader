use super::*;

impl Store {
    #[cfg(test)]
    pub async fn flush_provider_marks(&mut self, remote: &ProviderClient) -> anyhow::Result<usize> {
        let pending = {
            let mut statement = self.connection.prepare(
                "SELECT remote_id,field,value,revision FROM pending_remote_marks WHERE workspace=?1 ORDER BY remote_id,field",
            )?;
            statement
                .query_map([&self.workspace], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut read_ids = Vec::new();
        let mut unread_ids = Vec::new();
        let mut starred = Vec::new();
        for (remote_id, field, value, _) in &pending {
            match field.as_str() {
                "is_read" if *value => read_ids.push(*remote_id),
                "is_read" => unread_ids.push(*remote_id),
                "is_starred" => starred.push((*remote_id, *value)),
                _ => {}
            }
        }
        remote.mark_entries_status(&read_ids, true).await?;
        remote.mark_entries_status(&unread_ids, false).await?;
        for (remote_id, value) in starred {
            remote.set_starred(remote_id, value).await?;
        }
        let transaction = self.connection.transaction()?;
        let mut completed = 0;
        for (remote_id, field, _, revision) in pending {
            completed += transaction.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1 AND remote_id=?2 AND field=?3 AND revision=?4",
                params![self.workspace, remote_id, field, revision],
            )?;
        }
        transaction.commit()?;
        Ok(completed)
    }

    pub fn remote_entry_id(&self, id: i64) -> anyhow::Result<Option<i64>> {
        Ok(self.connection.query_row(
            "SELECT a.remote_id FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.id=?1 AND f.workspace=?2",
            params![id, self.workspace],
            |row| row.get(0),
        )?)
    }

    pub fn remote_feed_id(&self, id: i64) -> anyhow::Result<Option<i64>> {
        Ok(self.connection.query_row(
            "SELECT remote_id FROM feeds WHERE id=?1 AND workspace=?2",
            params![id, self.workspace],
            |row| row.get(0),
        )?)
    }

    /// Feed fields needed to remove it from the active provider as well.
    pub fn feed_removal_info(&self, id: i64) -> anyhow::Result<(Option<i64>, String, String)> {
        Ok(self.connection.query_row(
            "SELECT remote_id, feed_url, COALESCE(source, 'local') FROM feeds WHERE id=?1 AND workspace=?2",
            params![id, self.workspace],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }

    pub fn pending_remote_marks(&self) -> anyhow::Result<Vec<PendingRemoteMark>> {
        let mut statement = self.connection.prepare(
            "SELECT remote_id,field,value,revision FROM pending_remote_marks WHERE workspace=?1 ORDER BY remote_id,field",
        )?;
        Ok(statement
            .query_map([&self.workspace], |row| {
                Ok(PendingRemoteMark {
                    remote_id: row.get(0)?,
                    field: row.get(1)?,
                    value: row.get(2)?,
                    revision: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn provider_sync_state(&self) -> anyhow::Result<Option<ProviderSyncState>> {
        let state = self.connection.query_row(
            "SELECT provider,account,sync_cursor,last_full_sync_at FROM remote_state WHERE workspace=?1",
            [&self.workspace],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?)),
        ).optional()?;
        Ok(state.map(
            |(provider, account, cursor, last_full_sync_at)| ProviderSyncState {
                provider,
                account,
                cursor: cursor.map(|value| SyncCursor { value }),
                last_full_sync_at,
            },
        ))
    }

    pub fn begin_provider_sync(
        &self,
        kind: ProviderKind,
        account: &str,
        cursor: Option<&SyncCursor>,
        last_full_sync_at: Option<&str>,
        reset: bool,
    ) -> anyhow::Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        if reset {
            transaction.execute(
                "DELETE FROM feeds WHERE workspace=?1 AND source IS NOT NULL AND source<>'local'",
                [&self.workspace],
            )?;
            transaction.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1",
                [&self.workspace],
            )?;
        }
        transaction.execute(
            "INSERT INTO remote_state(workspace,provider,account,sync_cursor,last_sync_at,last_full_sync_at)
             VALUES(?1,?2,?3,?4,NULL,?5)
             ON CONFLICT(workspace) DO UPDATE SET provider=excluded.provider,account=excluded.account,
                sync_cursor=excluded.sync_cursor,last_full_sync_at=excluded.last_full_sync_at",
            params![self.workspace, kind.key(), account, cursor.map(|value| value.value.as_str()), last_full_sync_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Replace an active provider workspace with a fully synchronized staging workspace.
    /// Keeping the old workspace visible until this transaction commits prevents an account
    /// switch from discarding its cached subscriptions when network synchronization fails.
    pub fn promote_provider_workspace(
        &mut self,
        provider: ProviderKind,
        staging_workspace: &str,
    ) -> anyhow::Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM feeds WHERE workspace=?1 AND source IS NOT NULL AND source<>'local'",
            [&self.workspace],
        )?;
        transaction.execute(
            "DELETE FROM pending_remote_marks WHERE workspace=?1",
            [&self.workspace],
        )?;
        transaction.execute(
            "DELETE FROM remote_state WHERE workspace=?1",
            [&self.workspace],
        )?;
        transaction.execute(
            "UPDATE feeds SET workspace=?1 WHERE workspace=?2 AND source=?3",
            params![self.workspace, staging_workspace, provider.key()],
        )?;
        transaction.execute(
            "UPDATE pending_remote_marks SET workspace=?1 WHERE workspace=?2",
            params![self.workspace, staging_workspace],
        )?;
        transaction.execute(
            "UPDATE remote_state SET workspace=?1 WHERE workspace=?2",
            params![self.workspace, staging_workspace],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn update_provider_sync_cursor(
        &self,
        cursor: Option<&SyncCursor>,
        full_sync_at: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.connection.execute(
            "UPDATE remote_state SET sync_cursor=?1,last_sync_at=?2,last_full_sync_at=COALESCE(?3,last_full_sync_at) WHERE workspace=?4",
            params![cursor.map(|value| value.value.as_str()), now, full_sync_at, self.workspace],
        )?;
        Ok(())
    }

    pub fn acknowledge_remote_marks(
        &mut self,
        marks: &[PendingRemoteMark],
    ) -> anyhow::Result<usize> {
        let transaction = self.connection.transaction()?;
        let mut completed = 0;
        for mark in marks {
            completed += transaction.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1 AND remote_id=?2 AND field=?3 AND revision=?4",
                params![self.workspace, mark.remote_id, mark.field, mark.revision],
            )?;
        }
        transaction.commit()?;
        Ok(completed)
    }

    #[cfg(test)]
    pub async fn sync_provider(
        &mut self,
        remote: &ProviderClient,
        kind: ProviderKind,
    ) -> anyhow::Result<usize> {
        let identity = remote.identity().await?;
        let feeds = remote.feeds().await?;
        let account = identity.account;
        let previous: Option<(String, String, Option<String>, Option<String>)> = self
            .connection
            .query_row(
                "SELECT provider,account,sync_cursor,last_full_sync_at FROM remote_state WHERE workspace=?1",
                [&self.workspace],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let account_changed = previous
            .as_ref()
            .is_some_and(|(_, old_account, _, _)| old_account != &account);
        let provider_changed = previous.as_ref().is_some_and(|(old_provider, _, _, _)| {
            !old_provider.is_empty() && old_provider != kind.key()
        });
        let reset = account_changed || provider_changed;
        if reset {
            self.connection.execute(
                "DELETE FROM feeds WHERE source=?1 AND workspace=?2",
                params![kind.key(), self.workspace],
            )?;
            self.connection.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1",
                [&self.workspace],
            )?;
        }
        let mut cursor = if reset {
            None
        } else {
            previous
                .as_ref()
                .and_then(|(_, _, cursor, _)| cursor.as_ref())
                .map(|value| SyncCursor {
                    value: value.clone(),
                })
        };
        let last_full_sync_at = if reset {
            None
        } else {
            previous
                .as_ref()
                .and_then(|(_, _, _, timestamp)| timestamp.as_deref())
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&chrono::Utc))
        };
        let mode = if reset || previous.is_none() || cursor.is_none() {
            SyncMode::Initial
        } else if !remote.capabilities().incremental_sync {
            SyncMode::Reconcile
        } else if last_full_sync_at
            .is_none_or(|last| chrono::Utc::now().signed_duration_since(last).num_days() >= 7)
        {
            SyncMode::Reconcile
        } else {
            SyncMode::Incremental
        };
        self.connection.execute(
            "INSERT INTO remote_state(workspace,provider,account,sync_cursor,last_sync_at,last_full_sync_at)
             VALUES(?1,?2,?3,?4,NULL,?5)
             ON CONFLICT(workspace) DO UPDATE SET provider=excluded.provider,account=excluded.account,
                sync_cursor=excluded.sync_cursor,last_full_sync_at=excluded.last_full_sync_at",
            params![
                self.workspace,
                kind.key(),
                account,
                cursor.as_ref().map(|value| value.value.as_str()),
                last_full_sync_at.map(|value| value.to_rfc3339())
            ],
        )?;
        self.flush_provider_marks(remote).await?;
        let feed_ids = self.save_remote_feeds(&feeds, kind)?;
        let mut saved = 0;
        let mut full_sync = matches!(mode, SyncMode::Initial | SyncMode::Reconcile);
        loop {
            let page = remote.entries_page(mode, cursor.as_ref()).await?;
            full_sync |= page.full_sync;
            saved += self
                .save_remote_entries(&page.entries, &feed_ids, kind)?
                .count;
            cursor = page.next_cursor;
            let now = chrono::Utc::now().to_rfc3339();
            let full_sync_at = (!page.has_more && full_sync).then_some(now.as_str());
            self.connection.execute(
                "UPDATE remote_state SET sync_cursor=?1,last_sync_at=?2,
                    last_full_sync_at=COALESCE(?3,last_full_sync_at) WHERE workspace=?4",
                params![
                    cursor.as_ref().map(|value| value.value.as_str()),
                    now,
                    full_sync_at,
                    self.workspace
                ],
            )?;
            if !page.has_more {
                break;
            }
            if cursor.is_none() {
                anyhow::bail!("Provider returned another sync page without a continuation cursor");
            }
        }
        Ok(saved)
    }

    pub fn save_remote_feeds(
        &mut self,
        feeds: &[RemoteFeed],
        kind: ProviderKind,
    ) -> anyhow::Result<HashMap<i64, i64>> {
        let mut seen = HashSet::new();
        for feed in feeds {
            seen.insert(feed.id);
            let language = if kind == ProviderKind::Miniflux {
                feed.language.as_deref()
            } else {
                None
            };
            let folder = feed
                .category
                .as_ref()
                .map(|category| category.title.as_str());
            self.connection.execute(
                "UPDATE articles SET auto_translated_title=NULL,auto_translated_title_lang=NULL,auto_translated_title_source_hash=NULL
                 WHERE feed_id IN (
                    SELECT id FROM feeds WHERE remote_id=?1 AND workspace=?2
                    AND COALESCE(language,'') != COALESCE(?3,'')
                 )",
                params![feed.id, self.workspace, language],
            )?;
            let updated = self.connection.execute(
                "UPDATE feeds SET feed_url=?1,title=?2,site_url=?3,folder=?4,source=?5,language=?6
                 WHERE remote_id=?7 AND workspace=?8",
                params![
                    feed.feed_url,
                    feed.title,
                    feed.site_url,
                    folder,
                    kind.key(),
                    language,
                    feed.id,
                    self.workspace
                ],
            )?;
            if updated == 0 {
                self.connection.execute(
                    "INSERT INTO feeds(feed_url,title,site_url,folder,source,remote_id,workspace,language)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(workspace,feed_url) DO UPDATE SET
                    title=excluded.title,site_url=excluded.site_url,folder=excluded.folder,
                    source=excluded.source,remote_id=excluded.remote_id,language=excluded.language",
                    params![
                        feed.feed_url,
                        feed.title,
                        feed.site_url,
                        folder,
                        kind.key(),
                        feed.id,
                        self.workspace,
                        language
                    ],
                )?;
            }
        }
        let existing = {
            let mut statement = self
                .connection
                .prepare("SELECT id,remote_id FROM feeds WHERE source=?1 AND workspace=?2")?;
            statement
                .query_map(params![kind.key(), self.workspace], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut result = HashMap::new();
        for (local_id, remote_id) in existing {
            if seen.contains(&remote_id) {
                result.insert(remote_id, local_id);
            } else {
                self.clear_pending_for_feed(local_id)?;
                self.connection.execute(
                    "DELETE FROM feeds WHERE id=?1 AND workspace=?2",
                    params![local_id, self.workspace],
                )?;
            }
        }
        Ok(result)
    }

    #[cfg(test)]
    pub fn save_remote_entries(
        &mut self,
        entries: &[RemoteEntry],
        feed_ids: &HashMap<i64, i64>,
        kind: ProviderKind,
    ) -> anyhow::Result<SavedRemoteEntries> {
        let prepared = Self::prepare_remote_entries(entries.to_vec(), kind)?;
        self.save_prepared_remote_entries(&prepared, feed_ids)
    }

    pub fn prepare_remote_entries(
        entries: Vec<RemoteEntry>,
        kind: ProviderKind,
    ) -> anyhow::Result<Vec<PreparedRemoteEntry>> {
        Self::prepare_remote_entries_with_revisions(entries, kind, &HashMap::new())
    }

    pub fn prepare_remote_entries_with_revisions(
        entries: Vec<RemoteEntry>,
        kind: ProviderKind,
        known_revisions: &HashMap<i64, String>,
    ) -> anyhow::Result<Vec<PreparedRemoteEntry>> {
        entries
            .into_iter()
            .map(|entry| {
                let remote_hash = hex::encode(Sha256::digest(entry.content.as_bytes()));
                if entry.status == "removed" {
                    return Ok(PreparedRemoteEntry {
                        guid: format!("{}:{}", kind.key(), entry.id),
                        entry,
                        remote_hash,
                        html: String::new(),
                        snippet: String::new(),
                        compressed_source: Vec::new(),
                    });
                }
                if known_revisions.get(&entry.id) == Some(&remote_hash) {
                    return Ok(PreparedRemoteEntry {
                        guid: format!("{}:{}", kind.key(), entry.id),
                        entry,
                        remote_hash,
                        html: String::new(),
                        snippet: String::new(),
                        compressed_source: Vec::new(),
                    });
                }
                let html = sanitize_html(&entry.content, entry.url.as_deref());
                let snippet = plain_text(&html).chars().take(280).collect();
                let compressed_source = compress_html(&entry.content)?;
                Ok(PreparedRemoteEntry {
                    guid: format!("{}:{}", kind.key(), entry.id),
                    entry,
                    remote_hash,
                    html,
                    snippet,
                    compressed_source,
                })
            })
            .collect()
    }

    pub fn save_prepared_remote_entries(
        &mut self,
        entries: &[PreparedRemoteEntry],
        feed_ids: &HashMap<i64, i64>,
    ) -> anyhow::Result<SavedRemoteEntries> {
        let mut saved = 0;
        let mut changed_article_ids = Vec::new();
        for batch in entries.chunks(100) {
            let transaction = self.connection.transaction()?;
            for prepared in batch {
                let entry = &prepared.entry;
                let Some(feed_id) = feed_ids.get(&entry.feed_id) else {
                    continue;
                };
                if entry.status == "removed" {
                    let existing_id: Option<i64> = transaction
                        .query_row(
                            "SELECT id FROM articles WHERE remote_id=?1 AND feed_id=?2",
                            params![entry.id, feed_id],
                            |row| row.get(0),
                        )
                        .optional()?;
                    transaction.execute(
                        "DELETE FROM articles WHERE remote_id=?1 AND feed_id=?2",
                        params![entry.id, feed_id],
                    )?;
                    if let Some(article_id) = existing_id {
                        changed_article_ids.push(article_id);
                    }
                    continue;
                }
                let remote_hash = &prepared.remote_hash;
                let is_read = entry.status == "read";
                let existing: Option<(
                    i64,
                    Option<String>,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    bool,
                    bool,
                )> = transaction
                    .query_row(
                        "SELECT id,remote_content_hash,title,url,author,published_at,is_read,is_starred
                         FROM articles WHERE feed_id=?1 AND guid=?2",
                        params![feed_id, prepared.guid],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get::<_, i64>(6)? != 0,
                                row.get::<_, i64>(7)? != 0,
                            ))
                        },
                    )
                    .optional()?;

                if let Some((
                    article_id,
                    stored_hash,
                    old_title,
                    old_url,
                    old_author,
                    old_published_at,
                    old_is_read,
                    old_is_starred,
                )) = existing
                {
                    let content_changed = stored_hash.as_deref() != Some(remote_hash.as_str());
                    let title_changed = old_title != entry.title;
                    let author_changed = old_author != entry.author;
                    let status_changed = old_is_read != is_read || old_is_starred != entry.starred;
                    let changed = content_changed
                        || title_changed
                        || old_url != entry.url
                        || author_changed
                        || old_published_at != entry.published_at
                        || status_changed;
                    if !changed {
                        continue;
                    }

                    if content_changed {
                        transaction.execute(
                            "UPDATE articles SET title=?1,url=?2,author=?3,published_at=?4,
                                snippet=?5,content_html=?6,
                                source_html=CASE WHEN ?6='' THEN source_html ELSE ?7 END,
                                content_revision=content_revision+1,
                                remote_content_hash=?8,source_revision=?8,is_read=?9,is_starred=?10,
                                extraction_pipeline_hash=NULL,extracted_html=NULL,
                                processed_html=NULL,processed_source_hash=NULL,processed_pipeline_hash=NULL,
                                translated_html=NULL,translated_title=NULL,translated_lang=NULL,
                                translation_source_hash=NULL,auto_translated_title=NULL,
                                auto_translated_title_lang=NULL,auto_translated_title_source_hash=NULL
                             WHERE id=?11",
                            params![
                                entry.title,
                                entry.url,
                                entry.author,
                                entry.published_at,
                                prepared.snippet,
                                prepared.html,
                                prepared.compressed_source,
                                remote_hash,
                                is_read,
                                entry.starred,
                                article_id,
                            ],
                        )?;
                    } else {
                        transaction.execute(
                            "UPDATE articles SET title=?1,url=?2,author=?3,published_at=?4,
                                is_read=?5,is_starred=?6,
                                content_revision=content_revision+CASE WHEN title IS NOT ?1 THEN 1 ELSE 0 END
                             WHERE id=?7",
                            params![
                                entry.title,
                                entry.url,
                                entry.author,
                                entry.published_at,
                                is_read,
                                entry.starred,
                                article_id,
                            ],
                        )?;
                    }
                    if content_changed || title_changed || author_changed {
                        sync_article_fts(&transaction, article_id)?;
                    }
                    if content_changed || title_changed || author_changed || status_changed {
                        changed_article_ids.push(article_id);
                    }
                    saved += 1;
                } else {
                    transaction.execute(
                            "INSERT INTO articles(feed_id,guid,title,url,author,published_at,snippet,
                            content_html,source_html,is_read,is_starred,remote_id,remote_content_hash,source_revision)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
                        params![
                            feed_id,
                            prepared.guid,
                            entry.title,
                            entry.url,
                            entry.author,
                            entry.published_at,
                            prepared.snippet,
                            prepared.html,
                            prepared.compressed_source,
                            is_read,
                            entry.starred,
                            entry.id,
                            remote_hash,
                        ],
                    )?;
                    let article_id = transaction.last_insert_rowid();
                    sync_article_fts(&transaction, article_id)?;
                    changed_article_ids.push(article_id);
                    saved += 1;
                }
            }
            transaction.commit()?;
        }
        Ok(SavedRemoteEntries {
            count: saved,
            changed_article_ids,
        })
    }
}
