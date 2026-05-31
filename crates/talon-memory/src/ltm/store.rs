//! `LtmStore` — SQLite-backed long-term memory store (task 2.5.2).
//!
//! One transaction writes a memory row, its FTS5 index (via trigger), and its
//! optional `sqlite-vec` embedding. Keyword retrieval (`search_text`) and vector
//! KNN (`search_vector`) are exposed as primitives; the RRF hybrid fusion that
//! composes them lands in task 2.5.8.

use rusqlite::{OptionalExtension, params};
use zerocopy::IntoBytes;

use crate::{Database, Memory, MemoryError};

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
        let category = mem.category.clone();
        let importance = i64::from(mem.importance);
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
                    "INSERT INTO memories (content, category, importance, entities)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![content, category, importance, entities],
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
                            accessed_at, decay_score, entities
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
                            m.accessed_at, m.decay_score, m.entities
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

/// Map a `memories` row (8 columns, in schema order) into a `Memory`.
fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let entities_json: String = row.get(7)?;
    let entities: Vec<String> = serde_json::from_str(&entities_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Memory {
        id: row.get(0)?,
        content: row.get(1)?,
        category: row.get(2)?,
        importance: row.get::<_, i64>(3)? as u8,
        created_at: row.get(4)?,
        accessed_at: row.get(5)?,
        decay_score: row.get::<_, f64>(6)? as f32,
        entities,
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
            "user_preference",
            4,
            vec!["dark mode".into()],
        );
        let id = s.insert(&mem, None).await.expect("insert");
        assert!(id > 0);

        let got = s.get(id).await.expect("get").expect("present");
        assert_eq!(got.id, id);
        assert_eq!(got.content, "user prefers dark mode");
        assert_eq!(got.category, "user_preference");
        assert_eq!(got.importance, 4);
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
                "fact",
                3,
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
        s.insert(&Memory::new("nothing relevant", "fact", 1, vec![]), None)
            .await
            .expect("insert");
        assert!(s.search_text("xyzzy", 10).await.expect("search").is_empty());
    }

    #[tokio::test]
    async fn insert_with_embedding_is_knn_searchable() {
        let s = store().await;
        let near = s
            .insert(
                &Memory::new("near", "fact", 3, vec![]),
                Some(&emb(0.9, 0.1)),
            )
            .await
            .expect("insert near");
        let _far = s
            .insert(&Memory::new("far", "fact", 3, vec![]), Some(&emb(0.0, 1.0)))
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
        s.insert(&Memory::new("no vector", "fact", 3, vec![]), None)
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
                &Memory::new("hybrid roundtrip", "fact", 5, vec!["x".into()]),
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
    async fn importance_out_of_range_is_rejected() {
        let s = store().await;
        let bad = Memory::new("too important", "fact", 9, vec![]);
        let err = s.insert(&bad, None).await;
        assert!(
            err.is_err(),
            "CHECK(importance BETWEEN 1 AND 5) should reject 9"
        );
    }
}
