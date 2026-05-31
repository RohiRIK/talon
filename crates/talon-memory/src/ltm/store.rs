//! `LtmStore` — SQLite-backed long-term memory store (task 2.5.2).
//!
//! One transaction writes a memory row, its FTS5 index (via trigger), and its
//! optional `sqlite-vec` embedding. Keyword retrieval (`search_text`) and vector
//! KNN (`search_vector`) are exposed as primitives; the RRF hybrid fusion that
//! composes them lands in task 2.5.8.

use rusqlite::{OptionalExtension, params};
use zerocopy::IntoBytes;

use crate::{Database, Memory, MemoryCategory, MemoryError};

/// Long-term memory store over the shared `talon.db`.
#[derive(Debug, Clone)]
pub struct LtmStore {
    db: Database,
}

impl LtmStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert a memory plus (optionally) its embedding in a single transaction.
    /// The FTS5 row is written automatically by the `memories_fts_insert` trigger.
    /// Returns the new memory id.
    ///
    /// `embedding`, when present, must be exactly 384 dimensions (the `vec0`
    /// table is declared `float[384]`); a mismatch surfaces as a SQLite error.
    pub async fn insert(
        &self,
        mem: &Memory,
        embedding: Option<&[f32]>,
    ) -> Result<i64, MemoryError> {
        let content = mem.content.clone();
        let category = mem.category.as_str().to_string();
        let importance = i64::from(mem.importance);
        let tags = serde_json::to_string(&mem.tags)?;
        let entities = serde_json::to_string(&mem.entities)?;
        let embedding = embedding.map(<[f32]>::to_vec);

        let id = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<i64> {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO memories (content, category, importance, tags, entities)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![content, category, importance, tags, entities],
                )?;
                let id = tx.last_insert_rowid();
                if let Some(emb) = embedding {
                    tx.execute(
                        "INSERT INTO vec_memories (rowid, embedding) VALUES (?1, ?2)",
                        params![id, emb.as_bytes()],
                    )?;
                }
                tx.commit()?;
                Ok(id)
            })
            .await??;
        Ok(id)
    }

    /// Fetch a memory by id. Returns `None` if it does not exist.
    pub async fn get(&self, id: i64) -> Result<Option<Memory>, MemoryError> {
        let mem = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Option<Memory>> {
                conn.query_row(
                    "SELECT id, content, category, importance, created_at,
                            accessed_at, decay_score, entities, tags
                     FROM memories WHERE id = ?1",
                    [id],
                    memory_from_row,
                )
                .optional()
            })
            .await??;
        Ok(mem)
    }

    /// FTS5 BM25 keyword search over memory content, best match first.
    pub async fn search_text(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let query = query.to_string();
        let limit = limit as i64;
        let rows = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Vec<Memory>> {
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.content, m.category, m.importance, m.created_at,
                            m.accessed_at, m.decay_score, m.entities, m.tags
                     FROM memories m
                     JOIN memories_fts f ON f.rowid = m.id
                     WHERE memories_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )?;
                stmt.query_map(params![query, limit], memory_from_row)?
                    .collect()
            })
            .await??;
        Ok(rows)
    }

    /// Read back a stored embedding by memory id. Returns `None` when the memory
    /// has no vector row. Used by the [`crate::Deduplicator`] to compute an exact
    /// cosine similarity against a candidate (KNN only yields L2 distance).
    pub async fn get_embedding(&self, id: i64) -> Result<Option<Vec<f32>>, MemoryError> {
        let emb = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Option<Vec<f32>>> {
                conn.query_row(
                    "SELECT embedding FROM vec_memories WHERE rowid = ?1",
                    [id],
                    |row| {
                        let bytes: Vec<u8> = row.get(0)?;
                        Ok(bytes
                            .chunks_exact(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect())
                    },
                )
                .optional()
            })
            .await??;
        Ok(emb)
    }

    /// Reinforce an existing memory on a dedup merge: raise its importance to
    /// `importance` if that is higher, and refresh `accessed_at`. Returns the
    /// number of rows updated (0 if `id` does not exist).
    pub async fn reinforce(&self, id: i64, importance: u8) -> Result<usize, MemoryError> {
        let importance = i64::from(importance);
        let n = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<usize> {
                conn.execute(
                    "UPDATE memories
                     SET importance = MAX(importance, ?1), accessed_at = unixepoch()
                     WHERE id = ?2",
                    params![importance, id],
                )
            })
            .await??;
        Ok(n)
    }

    /// Recompute every memory's `decay_score` from how long ago it was accessed,
    /// in a single transaction. The score is [`crate::decay::decay_factor`] of
    /// `now_unix - accessed_at` under `half_life_days`. Returns the row count.
    pub async fn apply_decay(
        &self,
        half_life_days: f64,
        now_unix: i64,
    ) -> Result<usize, MemoryError> {
        let n = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<usize> {
                let tx = conn.transaction()?;
                let rows: Vec<(i64, i64)> = {
                    let mut stmt = tx.prepare("SELECT id, accessed_at FROM memories")?;
                    stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut updated = 0usize;
                for (id, accessed_at) in &rows {
                    let score = crate::decay::decay_factor(now_unix - accessed_at, half_life_days);
                    updated += tx.execute(
                        "UPDATE memories SET decay_score = ?1 WHERE id = ?2",
                        params![f64::from(score), id],
                    )?;
                }
                tx.commit()?;
                Ok(updated)
            })
            .await??;
        Ok(n)
    }

    /// sqlite-vec brute-force KNN. Returns `(memory_id, distance)` nearest first.
    pub async fn search_vector(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(i64, f32)>, MemoryError> {
        let emb = embedding.to_vec();
        let k = k as i64;
        let rows = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Vec<(i64, f32)>> {
                let mut stmt = conn.prepare(
                    "SELECT rowid, distance
                     FROM vec_memories
                     WHERE embedding MATCH ?1 AND k = ?2
                     ORDER BY distance",
                )?;
                stmt.query_map(params![emb.as_bytes(), k], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)? as f32))
                })?
                .collect()
            })
            .await??;
        Ok(rows)
    }
}

/// Map a `memories` row (9 columns, in the SELECT order used above) into a `Memory`.
fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let category: String = row.get(2)?;
    let entities: Vec<String> = json_array(row, 7)?;
    let tags: Vec<String> = json_array(row, 8)?;
    Ok(Memory {
        id: row.get(0)?,
        content: row.get(1)?,
        category: MemoryCategory::from_label(&category),
        importance: row.get::<_, i64>(3)? as u8,
        created_at: row.get(4)?,
        accessed_at: row.get(5)?,
        decay_score: row.get::<_, f64>(6)? as f32,
        tags,
        entities,
    })
}

/// Read a JSON-array text column into a `Vec<String>`.
fn json_array(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Vec<String>> {
    let raw: String = row.get(idx)?;
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn store() -> LtmStore {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("schema");
        LtmStore::new(db)
    }

    /// 384-dim embedding with two distinguishing leading components.
    fn emb(a: f32, b: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 384];
        v[0] = a;
        v[1] = b;
        v
    }

    #[tokio::test]
    async fn insert_and_get_roundtrips() {
        let s = store().await;
        let mem = Memory::new(
            "user prefers dark mode",
            MemoryCategory::UserPreference,
            4,
            vec!["ui".into()],
            vec!["dark mode".into()],
        );
        let id = s.insert(&mem, None).await.expect("insert");
        assert!(id > 0);

        let got = s.get(id).await.expect("get").expect("present");
        assert_eq!(got.id, id);
        assert_eq!(got.content, "user prefers dark mode");
        assert_eq!(got.category, MemoryCategory::UserPreference);
        assert_eq!(got.importance, 4);
        assert_eq!(got.tags, vec!["ui".to_string()]);
        assert_eq!(got.entities, vec!["dark mode".to_string()]);
        // DB-assigned defaults.
        assert!(got.created_at > 0);
        assert!((got.decay_score - 1.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let s = store().await;
        assert!(s.get(999).await.expect("get").is_none());
    }

    #[tokio::test]
    async fn insert_populates_fts_index() {
        let s = store().await;
        s.insert(
            &Memory::new(
                "the deploy script lives in scripts/release.sh",
                MemoryCategory::Fact,
                3,
                vec![],
                vec![],
            ),
            None,
        )
        .await
        .expect("insert");

        let hits = s.search_text("deploy", 10).await.expect("search");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("release.sh"));
    }

    #[tokio::test]
    async fn search_text_returns_empty_on_no_match() {
        let s = store().await;
        s.insert(
            &Memory::new("nothing relevant", MemoryCategory::Fact, 1, vec![], vec![]),
            None,
        )
        .await
        .expect("insert");
        assert!(s.search_text("xyzzy", 10).await.expect("search").is_empty());
    }

    #[tokio::test]
    async fn insert_with_embedding_is_knn_searchable() {
        let s = store().await;
        let near = s
            .insert(
                &Memory::new("near", MemoryCategory::Fact, 3, vec![], vec![]),
                Some(&emb(0.9, 0.1)),
            )
            .await
            .expect("insert near");
        let _far = s
            .insert(
                &Memory::new("far", MemoryCategory::Fact, 3, vec![], vec![]),
                Some(&emb(0.0, 1.0)),
            )
            .await
            .expect("insert far");

        let results = s.search_vector(&emb(1.0, 0.0), 1).await.expect("knn");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].0, near,
            "nearest to [1,0] should be the 'near' row"
        );
    }

    #[tokio::test]
    async fn insert_without_embedding_has_no_vector() {
        let s = store().await;
        s.insert(
            &Memory::new("no vector", MemoryCategory::Fact, 3, vec![], vec![]),
            None,
        )
        .await
        .expect("insert");
        // KNN over an empty vec table yields nothing.
        let results = s.search_vector(&emb(1.0, 0.0), 5).await.expect("knn");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn fact_embedding_and_fts_commit_together() {
        let s = store().await;
        let id = s
            .insert(
                &Memory::new(
                    "hybrid roundtrip",
                    MemoryCategory::Fact,
                    5,
                    vec![],
                    vec!["x".into()],
                ),
                Some(&emb(0.5, 0.5)),
            )
            .await
            .expect("insert");

        // All three artifacts exist: row, FTS hit, and vector hit — same id.
        assert!(s.get(id).await.expect("get").is_some());
        assert_eq!(s.search_text("hybrid", 10).await.expect("fts")[0].id, id);
        assert_eq!(
            s.search_vector(&emb(0.5, 0.5), 1).await.expect("knn")[0].0,
            id
        );
    }

    #[tokio::test]
    async fn get_embedding_roundtrips_stored_vector() {
        let s = store().await;
        let id = s
            .insert(
                &Memory::new("vec", MemoryCategory::Fact, 3, vec![], vec![]),
                Some(&emb(0.25, 0.75)),
            )
            .await
            .expect("insert");
        let got = s
            .get_embedding(id)
            .await
            .expect("get_embedding")
            .expect("present");
        assert_eq!(got.len(), 384);
        assert!((got[0] - 0.25).abs() < f32::EPSILON);
        assert!((got[1] - 0.75).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn get_embedding_absent_returns_none() {
        let s = store().await;
        let id = s
            .insert(
                &Memory::new("no vec", MemoryCategory::Fact, 3, vec![], vec![]),
                None,
            )
            .await
            .expect("insert");
        assert!(s.get_embedding(id).await.expect("get_embedding").is_none());
    }

    #[tokio::test]
    async fn reinforce_raises_importance_to_max_only() {
        let s = store().await;
        let id = s
            .insert(
                &Memory::new("reinforce", MemoryCategory::Fact, 3, vec![], vec![]),
                None,
            )
            .await
            .expect("insert");

        assert_eq!(s.reinforce(id, 5).await.expect("reinforce"), 1);
        assert_eq!(
            s.get(id).await.expect("get").expect("present").importance,
            5
        );

        // A lower importance must not lower the existing value.
        assert_eq!(s.reinforce(id, 2).await.expect("reinforce"), 1);
        assert_eq!(
            s.get(id).await.expect("get").expect("present").importance,
            5
        );
    }

    #[tokio::test]
    async fn reinforce_missing_id_updates_nothing() {
        let s = store().await;
        assert_eq!(s.reinforce(999, 4).await.expect("reinforce"), 0);
    }

    #[tokio::test]
    async fn importance_out_of_range_is_rejected() {
        let s = store().await;
        let bad = Memory::new("too important", MemoryCategory::Fact, 9, vec![], vec![]);
        let err = s.insert(&bad, None).await;
        assert!(
            err.is_err(),
            "CHECK(importance BETWEEN 1 AND 5) should reject 9"
        );
    }
}
