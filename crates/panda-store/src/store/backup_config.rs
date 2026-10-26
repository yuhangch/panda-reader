use super::Store;
use serde::{Deserialize, Serialize};

/// Secret-free local subscription settings used by Config Only backups.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupFeed {
    pub feed_url: String,
    pub title: String,
    pub custom_title: Option<String>,
    pub folder: Option<String>,
    pub auto_translate_titles: bool,
}

impl Store {
    pub fn export_local_backup_feeds(&self) -> anyhow::Result<Vec<BackupFeed>> {
        let mut statement = self.connection.prepare(
            "SELECT feed_url,title,custom_title,folder,auto_translate_titles
             FROM feeds WHERE workspace='local' AND source='local'
             ORDER BY feed_url COLLATE NOCASE",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(BackupFeed {
                    feed_url: row.get(0)?,
                    title: row.get(1)?,
                    custom_title: row.get(2)?,
                    folder: row.get(3)?,
                    auto_translate_titles: row.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Merge Config Only feeds without deleting subscriptions, articles or marks.
    pub fn merge_local_backup_feeds(&mut self, feeds: &[BackupFeed]) -> anyhow::Result<usize> {
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let mut changed = 0;
            for feed in feeds {
                let feed_url = super::normalize_http_url(&feed.feed_url)?;
                anyhow::ensure!(
                    feed_url.len() <= 16_384,
                    "backup contains an invalid feed URL"
                );
                let exists: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM feeds WHERE workspace='local' AND feed_url=?1 AND source='local')",
                    [&feed_url], |row| row.get(0),
                )?;
                if exists {
                    changed += self.connection.execute(
                        "UPDATE feeds SET title=?1,custom_title=?2,folder=?3,auto_translate_titles=?4
                         WHERE workspace='local' AND feed_url=?5 AND source='local'",
                        rusqlite::params![feed.title, feed.custom_title, feed.folder, i64::from(feed.auto_translate_titles), feed_url],
                    )?;
                } else {
                    self.connection.execute(
                        "INSERT INTO feeds(feed_url,title,custom_title,folder,source,workspace,auto_translate_titles)
                         VALUES(?1,?2,?3,?4,'local','local',?5)",
                        rusqlite::params![feed_url, feed.title, feed.custom_title, feed.folder, i64::from(feed.auto_translate_titles)],
                    )?;
                    changed += 1;
                }
            }
            Ok::<usize, anyhow::Error>(changed)
        })();
        match result {
            Ok(changed) => {
                self.connection.execute_batch("COMMIT")?;
                Ok(changed)
            }
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn config_merge_keeps_existing_articles_and_unmentioned_feeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reader.sqlite3");
        Store::migrate(&path).unwrap();
        let mut store = Store::open(&path).unwrap();
        store.connection.execute(
            "INSERT INTO feeds(feed_url,title,source,workspace) VALUES('https://keep.test/rss','Keep','local','local')",
            [],
        ).unwrap();
        let keep_id: i64 = store
            .connection
            .query_row(
                "SELECT id FROM feeds WHERE feed_url='https://keep.test/rss'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        store.connection.execute(
            "INSERT INTO articles(feed_id,guid,title,is_read,is_starred) VALUES(?1,'article-1','Existing article',1,1)",
            [keep_id],
        ).unwrap();

        store
            .merge_local_backup_feeds(&[
                BackupFeed {
                    feed_url: "https://keep.test/rss".into(),
                    title: "Renamed".into(),
                    custom_title: Some("Custom".into()),
                    folder: Some("News".into()),
                    auto_translate_titles: true,
                },
                BackupFeed {
                    feed_url: "https://new.test/rss".into(),
                    title: "New".into(),
                    custom_title: None,
                    folder: None,
                    auto_translate_titles: false,
                },
            ])
            .unwrap();
        let (title, read, starred): (String, i64, i64) = store.connection.query_row(
            "SELECT f.custom_title,a.is_read,a.is_starred FROM feeds f JOIN articles a ON a.feed_id=f.id WHERE f.id=?1",
            [keep_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(title, "Custom");
        assert_eq!((read, starred), (1, 1));
        let feed_count: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM feeds WHERE workspace='local'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(feed_count, 2);
    }
}
