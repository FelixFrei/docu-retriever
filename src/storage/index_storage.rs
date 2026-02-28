use std::{path::Path, sync::Arc};

use arrow_array::{
    types::Float32Type, Array, FixedSizeListArray, Float32Array, Float64Array, RecordBatch,
    RecordBatchIterator, RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::{
    database::CreateTableMode,
    index::Index,
    query::{ExecutableQuery, QueryBase, Select},
    Error as LanceError,
};

use crate::models::{AppError, ChunkMetadata, ChunkRecord, QueryResult};

const TABLE_NAME: &str = "chunks";

#[derive(Debug, Default)]
pub struct IndexStorage;

impl IndexStorage {
    pub async fn persist(
        &self,
        db_path: &Path,
        records: &[ChunkRecord],
        dimension: usize,
    ) -> Result<(), AppError> {
        if dimension == 0 {
            return Err(AppError::storage(
                "Embedding dimension must be greater than zero",
            ));
        }
        if records.is_empty() {
            return Err(AppError::storage("Cannot persist empty record set"));
        }

        for record in records {
            if record.embedding.len() != dimension {
                return Err(AppError::storage(format!(
                    "Invalid embedding length for chunk '{}': expected {}, got {}",
                    record.chunk_id,
                    dimension,
                    record.embedding.len()
                )));
            }
        }

        let db = lancedb::connect(&db_path.to_string_lossy())
            .execute()
            .await
            .map_err(lance_to_storage)?;

        let schema = Arc::new(Schema::new(vec![
            Field::new("chunk_id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("source_file", DataType::Utf8, false),
            Field::new("original_file", DataType::Utf8, false),
            Field::new("section", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dimension as i32,
                ),
                false,
            ),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from_iter_values(
                    records.iter().map(|r| r.chunk_id.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    records.iter().map(|r| r.content.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    records.iter().map(|r| r.metadata.source_file.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    records.iter().map(|r| r.metadata.original_file.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    records.iter().map(|r| r.metadata.section.as_str()),
                )),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                        records.iter().map(|r| {
                            Some(r.embedding.iter().copied().map(Some).collect::<Vec<_>>())
                        }),
                        dimension as i32,
                    ),
                ),
            ],
        )
        .map_err(AppError::internal)?;

        let reader: Box<dyn RecordBatchReader + Send> = Box::new(RecordBatchIterator::new(
            vec![Ok(batch)].into_iter(),
            schema,
        ));

        let table = db
            .create_table(TABLE_NAME, reader)
            .mode(CreateTableMode::Overwrite)
            .execute()
            .await
            .map_err(lance_to_storage)?;

        if records.len() >= 256 {
            table
                .create_index(&["vector"], Index::Auto)
                .execute()
                .await
                .map_err(lance_to_storage)?;
        }

        Ok(())
    }

    pub async fn vector_dimension(&self, db_path: &Path) -> Result<usize, AppError> {
        let table = self.open_chunks_table(db_path).await?;
        let schema = table.schema().await.map_err(lance_to_storage)?;
        let field = schema.field_with_name("vector").map_err(|err| {
            AppError::storage(format!(
                "Index schema does not contain 'vector' column: {err}"
            ))
        })?;

        match field.data_type() {
            DataType::FixedSizeList(_, dim) => Ok(*dim as usize),
            other => Err(AppError::storage(format!(
                "Unsupported vector column type: {other:?}"
            ))),
        }
    }

    pub async fn search(
        &self,
        db_path: &Path,
        query_embedding: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<QueryResult>, AppError> {
        if query_embedding.is_empty() {
            return Err(AppError::invalid_input("Query embedding must not be empty"));
        }
        if top_k == 0 {
            return Err(AppError::invalid_input("top-k must be greater than zero"));
        }

        let table = self.open_chunks_table(db_path).await?;
        let mut stream = table
            .vector_search(query_embedding)
            .map_err(lance_to_storage)?
            .distance_type(lancedb::DistanceType::Cosine)
            .select(Select::columns(&[
                "chunk_id",
                "content",
                "source_file",
                "original_file",
                "section",
                "_distance",
            ]))
            .limit(top_k)
            .execute()
            .await
            .map_err(lance_to_storage)?;

        let mut out = Vec::new();
        while let Some(batch) = stream.try_next().await.map_err(lance_to_storage)? {
            parse_batch(&batch, &mut out)?;
        }
        Ok(out)
    }

    async fn open_chunks_table(&self, db_path: &Path) -> Result<lancedb::Table, AppError> {
        if !db_path.exists() {
            return Err(index_not_found(db_path));
        }

        let db = lancedb::connect(&db_path.to_string_lossy())
            .execute()
            .await
            .map_err(lance_to_storage)?;

        db.open_table(TABLE_NAME)
            .execute()
            .await
            .map_err(|err| match err {
                LanceError::TableNotFound { .. } => index_not_found(db_path),
                other => lance_to_storage(other),
            })
    }
}

fn parse_batch(batch: &RecordBatch, out: &mut Vec<QueryResult>) -> Result<(), AppError> {
    let chunk_id = utf8_column(batch, "chunk_id")?;
    let content = utf8_column(batch, "content")?;
    let source_file = utf8_column(batch, "source_file")?;
    let original_file = utf8_column(batch, "original_file")?;
    let section = utf8_column(batch, "section")?;
    let distance_col = batch.column_by_name("_distance").ok_or_else(|| {
        AppError::storage("LanceDB result did not include '_distance' column".to_string())
    })?;

    for row in 0..batch.num_rows() {
        let distance = distance_at(distance_col.as_ref(), row)?;
        let score = 1.0f32 / (1.0f32 + distance.max(0.0));
        out.push(QueryResult {
            chunk_id: chunk_id.value(row).to_string(),
            score,
            content: content.value(row).to_string(),
            metadata: ChunkMetadata {
                source_file: source_file.value(row).to_string(),
                original_file: original_file.value(row).to_string(),
                section: section.value(row).to_string(),
            },
        });
    }

    Ok(())
}

fn utf8_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray, AppError> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| AppError::storage(format!("Missing result column '{name}'")))?;
    col.as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| AppError::storage(format!("Column '{name}' is not Utf8")))
}

fn distance_at(array: &dyn Array, row: usize) -> Result<f32, AppError> {
    if let Some(values) = array.as_any().downcast_ref::<Float32Array>() {
        if values.is_null(row) {
            return Err(AppError::storage(
                "Distance column contains NULL".to_string(),
            ));
        }
        return Ok(values.value(row));
    }

    if let Some(values) = array.as_any().downcast_ref::<Float64Array>() {
        if values.is_null(row) {
            return Err(AppError::storage(
                "Distance column contains NULL".to_string(),
            ));
        }
        return Ok(values.value(row) as f32);
    }

    Err(AppError::storage(format!(
        "Unsupported distance column type: {:?}",
        array.data_type()
    )))
}

fn index_not_found(db_path: &Path) -> AppError {
    AppError::index_not_found(format!(
        "The index at {} could not be found. Please run the 'index' command.",
        db_path.display()
    ))
}

fn lance_to_storage(err: LanceError) -> AppError {
    AppError::storage(format!("LanceDB error: {err}"))
}
