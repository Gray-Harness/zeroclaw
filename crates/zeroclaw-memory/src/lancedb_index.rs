//! LanceDB-backed vector / full-text / hybrid index for `SqliteMemory`.
//!
//! This module is gated behind the `memory-lancedb` feature. It maintains a
//! derived LanceDB table that accelerates `SqliteMemory::recall` by replacing
//! the brute-force cosine scan with ANN vector search + BM25 full-text search
//! + scalar filtering.
//!
//! ## Source of truth
//!
//! The SQLite `memories` table remains the canonical store. The LanceDB table
//! is a disposable, rebuildable index. Any mismatch can be repaired by
//! dropping the LanceDB directory and letting the next write or an explicit
//! reindex repopulate it.
//!
//! ## Lazy table creation
//!
//! The table is created on the first `upsert` so the vector dimension is known
//! and no placeholder rows with the wrong width are left behind.

use anyhow::Context;
use arrow_array::{
    ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::index::scalar::FullTextSearchQuery;
use lancedb::index::{Index, scalar::FtsIndexBuilder, vector::IvfHnswSqIndexBuilder};
use lancedb::query::{ExecutableQuery, QueryBase};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

/// A single row in the derived LanceDB index.
#[derive(Clone, Debug)]
pub struct IndexEntry {
    pub id: String,
    pub key: String,
    pub content: String,
    pub vector: Vec<f32>,
    pub agent_id: Option<String>,
    pub namespace: String,
    pub session_id: Option<String>,
    pub category: String,
    pub superseded_by: Option<String>,
}

/// Scored identifier returned by a search.
#[derive(Clone, Debug)]
pub struct ScoredId {
    pub id: String,
    pub score: f32,
}

/// Search request against the LanceDB index.
#[derive(Clone, Debug, Default)]
pub struct SearchRequest {
    pub query_vector: Option<Vec<f32>>,
    pub query_text: Option<String>,
    pub limit: usize,
    pub agent_id: Option<String>,
    pub namespace: Option<String>,
    pub session_id: Option<String>,
    pub category: Option<String>,
    pub superseded: bool,
}

/// LanceDB-backed vector / FTS / hybrid index.
pub struct LanceDbVectorIndex {
    db: lancedb::Connection,
    table: Mutex<Option<lancedb::Table>>,
    /// Set to `true` when a newly-created table still needs its initial vector
    /// and FTS indexes built. Guarded by the table mutex in `ensure_table` and
    /// `ensure_indexes`.
    needs_index_creation: AtomicBool,
}

impl LanceDbVectorIndex {
    /// Open the index at `<db_dir>/lancedb_index/<namespace>`.
    ///
    /// `db_dir` is the directory containing the SQLite brain.db file. The
    /// underlying LanceDB table is created lazily on the first write so its
    /// vector dimension matches the embedding model in use.
    pub async fn open(db_dir: &Path, namespace: &str) -> anyhow::Result<Self> {
        let index_dir = db_dir.join("lancedb_index").join(namespace);
        std::fs::create_dir_all(&index_dir)
            .with_context(|| format!("create LanceDB index dir: {}", index_dir.display()))?;

        let index_dir_str = index_dir.to_str().with_context(|| {
            format!(
                "LanceDB index path is not valid UTF-8: {}",
                index_dir.display()
            )
        })?;
        let db = lancedb::connect(index_dir_str)
            .execute()
            .await
            .with_context(|| format!("connect LanceDB at {}", index_dir.display()))?;

        let (table, needs_index_creation) = match db.open_table("memories").execute().await {
            Ok(table) => (Some(table), false),
            Err(lancedb::Error::TableNotFound { .. }) => (None, true),
            Err(e) => return Err(e.into()),
        };

        Ok(Self {
            db,
            table: Mutex::new(table),
            needs_index_creation: AtomicBool::new(needs_index_creation),
        })
    }

    fn schema(dim: i32) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("key", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("agent_id", DataType::Utf8, true),
            Field::new("namespace", DataType::Utf8, false),
            Field::new("session_id", DataType::Utf8, true),
            Field::new("category", DataType::Utf8, false),
            Field::new("superseded_by", DataType::Utf8, true),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
                true,
            ),
        ]))
    }

    async fn ensure_table(
        &self,
        dim: i32,
    ) -> anyhow::Result<tokio::sync::MutexGuard<'_, Option<lancedb::Table>>> {
        let mut guard = self.table.lock().await;
        if guard.is_none() {
            let schema = Self::schema(dim);
            let batches = RecordBatchIterator::new(Vec::new().into_iter(), schema.clone());
            let table = self
                .db
                .create_table(
                    "memories",
                    Box::new(batches) as Box<dyn RecordBatchReader + Send>,
                )
                .execute()
                .await
                .context("create empty LanceDB memories table")?;
            self.needs_index_creation.store(true, Ordering::Relaxed);
            *guard = Some(table);
        }
        Ok(guard)
    }

    async fn create_indexes_on_table(table: &lancedb::Table) -> anyhow::Result<()> {
        // Build vector index if not present. We use IVF_HNSW_SQ as a balanced
        // default: HNSW graph gives high recall, scalar quantization keeps
        // memory modest.
        table
            .create_index(
                &["vector"],
                Index::IvfHnswSq(
                    IvfHnswSqIndexBuilder::default().distance_type(lancedb::DistanceType::Cosine),
                ),
            )
            .execute()
            .await
            .context("create vector index")?;

        table
            .create_index(&["content"], Index::FTS(FtsIndexBuilder::default()))
            .execute()
            .await
            .context("create FTS index")?;

        Ok(())
    }

    /// Ensure vector and FTS indexes exist.
    ///
    /// No-op if the table has not been created yet; indexes are built
    /// automatically on first `upsert`.
    pub async fn ensure_indexes(&self) -> anyhow::Result<()> {
        if !self.needs_index_creation.load(Ordering::Relaxed) {
            return Ok(());
        }
        let guard = self.table.lock().await;
        let Some(table) = guard.as_ref() else {
            return Ok(());
        };
        Self::create_indexes_on_table(table).await?;
        self.needs_index_creation.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Insert or replace entries in the index.
    ///
    /// Existing rows with matching `key` are overwritten so repeated indexing
    /// of the same source file is idempotent. The table is created on the first
    /// non-empty upsert using the dimension of the first entry.
    pub async fn upsert(&self, entries: &[IndexEntry]) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let dim = entries
            .first()
            .map(|e| e.vector.len() as i32)
            .filter(|&d| d > 0)
            .with_context(|| "cannot upsert LanceDB index: entries have empty vectors")?;

        let guard = self.ensure_table(dim).await?;
        let table = guard.as_ref().context("LanceDB table not available")?;
        let schema = Self::schema(dim);

        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        let contents: Vec<&str> = entries.iter().map(|e| e.content.as_str()).collect();
        let agent_ids: Vec<Option<&str>> = entries.iter().map(|e| e.agent_id.as_deref()).collect();
        let namespaces: Vec<&str> = entries.iter().map(|e| e.namespace.as_str()).collect();
        let session_ids: Vec<Option<&str>> =
            entries.iter().map(|e| e.session_id.as_deref()).collect();
        let categories: Vec<&str> = entries.iter().map(|e| e.category.as_str()).collect();
        let superseded_bys: Vec<Option<&str>> =
            entries.iter().map(|e| e.superseded_by.as_deref()).collect();

        let flat: Vec<f32> = entries
            .iter()
            .flat_map(|e| e.vector.iter().copied())
            .collect();
        let values = Arc::new(Float32Array::from(flat));
        let vectors = FixedSizeListArray::try_new(
            Arc::new(Field::new("item", DataType::Float32, true)),
            dim,
            values,
            None,
        )
        .context("build vector array")?;

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)) as ArrayRef,
                Arc::new(StringArray::from(keys)) as ArrayRef,
                Arc::new(StringArray::from(contents)) as ArrayRef,
                Arc::new(StringArray::from(agent_ids)) as ArrayRef,
                Arc::new(StringArray::from(namespaces)) as ArrayRef,
                Arc::new(StringArray::from(session_ids)) as ArrayRef,
                Arc::new(StringArray::from(categories)) as ArrayRef,
                Arc::new(StringArray::from(superseded_bys)) as ArrayRef,
                Arc::new(vectors) as ArrayRef,
            ],
        )
        .context("build upsert batch")?;

        table
            .add(
                Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()))
                    as Box<dyn RecordBatchReader + Send>,
            )
            .execute()
            .await
            .context("add rows to LanceDB")?;

        // Build indexes on the first successful upsert. We drop the table
        // guard first because ensure_indexes acquires the same mutex.
        drop(guard);
        self.ensure_indexes().await?;

        Ok(())
    }

    /// Delete rows whose `key` starts with `prefix`.
    pub async fn delete_key_prefix(&self, prefix: &str) -> anyhow::Result<()> {
        let guard = self.table.lock().await;
        let Some(table) = guard.as_ref() else {
            return Ok(());
        };
        // LanceDB where clauses use SQL syntax; strings must be quoted.
        let escaped = prefix.replace('\\', "\\\\").replace('\'', "\\'");
        let sql = format!("key LIKE '{escaped}%'");
        table
            .delete(&sql)
            .await
            .with_context(|| format!("delete LanceDB rows with key prefix {prefix}"))?;
        Ok(())
    }

    /// Search the index.
    pub async fn search(&self, req: SearchRequest) -> anyhow::Result<Vec<ScoredId>> {
        let guard = self.table.lock().await;
        let Some(table) = guard.as_ref() else {
            return Ok(Vec::new());
        };

        let builder = table.query();

        // Compose filters.
        let mut filters: Vec<String> = Vec::new();
        if !req.superseded {
            filters.push("superseded_by IS NULL".to_string());
        }
        if let Some(ns) = req.namespace {
            filters.push(format!(
                "namespace = '{escaped}'",
                escaped = escape_sql(&ns)
            ));
        }
        if let Some(agent) = req.agent_id {
            filters.push(format!(
                "(agent_id = '{escaped}' OR agent_id IS NULL)",
                escaped = escape_sql(&agent)
            ));
        }
        if let Some(sid) = req.session_id {
            filters.push(format!(
                "session_id = '{escaped}'",
                escaped = escape_sql(&sid)
            ));
        }
        if let Some(cat) = req.category {
            filters.push(format!(
                "category = '{escaped}'",
                escaped = escape_sql(&cat)
            ));
        }
        let filter_sql = if filters.is_empty() {
            None
        } else {
            Some(filters.join(" AND "))
        };

        let mut has_vector = false;
        let mut has_text = false;
        let limit = req.limit.max(1);

        // LanceDB query building is typed: nearest_to moves from Query to VectorQuery.
        // Keep the three query shapes separate so the types stay simple.
        let stream = if let Some(vec) = req.query_vector {
            let mut vq = builder.nearest_to(vec).context("set query vector")?;
            has_vector = true;
            if let Some(text) = req.query_text {
                vq = vq.full_text_search(FullTextSearchQuery::new(text));
                has_text = true;
            }
            if let Some(f) = filter_sql {
                vq = vq.only_if(f);
            }
            vq.limit(limit)
                .execute()
                .await
                .context("execute LanceDB vector query")?
        } else if let Some(text) = req.query_text {
            let mut tq = builder.full_text_search(FullTextSearchQuery::new(text));
            has_text = true;
            if let Some(f) = filter_sql {
                tq = tq.only_if(f);
            }
            tq.limit(limit)
                .execute()
                .await
                .context("execute LanceDB FTS query")?
        } else {
            let mut q = builder;
            if let Some(f) = filter_sql {
                q = q.only_if(f);
            }
            q.limit(limit)
                .execute()
                .await
                .context("execute LanceDB scalar query")?
        };

        let batches = stream
            .try_collect::<Vec<_>>()
            .await
            .context("collect LanceDB results")?;

        let mut scored = Vec::new();
        for batch in &batches {
            let id_col = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .context("id column missing or wrong type")?;

            // Distance / score column name depends on query type.
            let score_col = batch
                .column_by_name("_distance")
                .or_else(|| batch.column_by_name("_score"))
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

            for i in 0..batch.num_rows() {
                let id = id_col.value(i).to_string();
                let score = score_col.as_ref().map(|c| c.value(i)).unwrap_or(0.0);
                // For pure vector search, lower cosine distance = better.
                // For FTS, higher BM25 score = better.
                // For hybrid, LanceDB returns a fused relevance; we keep it as-is.
                let normalized = if has_vector && !has_text {
                    1.0 - score // convert cosine distance to similarity
                } else {
                    score
                };
                scored.push(ScoredId {
                    id,
                    score: normalized,
                });
            }
        }

        // Sort by descending score (similar to gray-memory's convention).
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(req.limit);
        Ok(scored)
    }

    /// Path to the LanceDB index directory for a given SQLite directory.
    pub fn index_path(db_dir: &Path, namespace: &str) -> PathBuf {
        db_dir.join("lancedb_index").join(namespace)
    }
}

fn escape_sql(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}
