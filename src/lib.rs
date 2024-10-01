use async_trait::async_trait;
use deadpool_postgres::{GenericClient, Pool};
use time::OffsetDateTime;
use tower_sessions_core::{
    session::{Id, Record},
    session_store, ExpiredDeletion, SessionStore,
};

#[derive(Debug, thiserror::Error)]
#[error("Pg session store error: {0}")]
pub enum Error {
    Pool(
        #[from]
        #[source]
        deadpool_postgres::PoolError,
    ),
    Pg(
        #[from]
        #[source]
        tokio_postgres::Error,
    ),
    Encode(
        #[from]
        #[source]
        rmp_serde::encode::Error,
    ),
    Decode(
        #[from]
        #[source]
        rmp_serde::decode::Error,
    ),
}

impl From<Error> for session_store::Error {
    fn from(e: Error) -> Self {
        Self::Backend(e.to_string())
    }
}

/// A PostgreSQL session store.
#[derive(Clone, Debug)]
pub struct PostgresStore {
    pool: Pool,
    schema_name: String,
    table_name: String,
}

impl PostgresStore {
    /// Create a new PostgreSQL store with the provided connection pool.
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            schema_name: "tower_sessions".to_string(),
            table_name: "session".to_string(),
        }
    }

    /// Set the session table schema name with the provided name.
    pub fn with_schema_name(mut self, schema_name: impl AsRef<str>) -> Result<Self, String> {
        let schema_name = schema_name.as_ref();
        if !is_valid_identifier(schema_name) {
            return Err(format!(
                "Invalid schema name '{}'. Schema names must start with a letter or underscore \
                 (including letters with diacritical marks and non-Latin letters).Subsequent \
                 characters can be letters, underscores, digits (0-9), or dollar signs ($).",
                schema_name
            ));
        }

        schema_name.clone_into(&mut self.schema_name);
        Ok(self)
    }

    /// Set the session table name with the provided name.
    pub fn with_table_name(mut self, table_name: impl AsRef<str>) -> Result<Self, String> {
        let table_name = table_name.as_ref();
        if !is_valid_identifier(table_name) {
            return Err(format!(
                "Invalid table name '{}'. Table names must start with a letter or underscore \
                 (including letters with diacritical marks and non-Latin letters).Subsequent \
                 characters can be letters, underscores, digits (0-9), or dollar signs ($).",
                table_name
            ));
        }

        table_name.clone_into(&mut self.table_name);
        Ok(self)
    }

    /// Migrate the session schema.
    pub async fn migrate(&self) -> Result<(), Error> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        let create_schema_query = format!(
            r#"create schema if not exists "{schema_name}""#,
            schema_name = self.schema_name,
        );

        // Concurrent create schema may fail due to duplicate key violations.
        //
        // This works around that by assuming the schema must exist on such an error.
        if let Err(err) = tx.execute(&create_schema_query, &[]).await {
            use tokio_postgres::error::SqlState;
            if matches!(
                err.code(),
                Some(&SqlState::DUPLICATE_SCHEMA | &SqlState::UNIQUE_VIOLATION)
            ) {
                return Ok(());
            }

            return Err(err.into());
        }

        let create_table_query = format!(
            r#"
            create table if not exists "{schema_name}"."{table_name}"
            (
                id text primary key not null,
                data bytea not null,
                expiry_date timestamptz not null
            )
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        tx.execute(&create_table_query, &[]).await?;

        tx.commit().await?;

        Ok(())
    }

    async fn id_exists(&self, conn: &impl GenericClient, id: &Id) -> Result<bool, Error> {
        let query = format!(
            r#"
            select exists(select 1 from "{schema_name}"."{table_name}" where id = $1)
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );

        Ok(conn.query_one(&query, &[&id.to_string()]).await?.get(0))
    }

    async fn save_with_conn(
        &self,
        conn: &impl GenericClient,
        record: &Record,
    ) -> Result<(), Error> {
        let query = format!(
            r#"
            insert into "{schema_name}"."{table_name}" (id, data, expiry_date)
            values ($1, $2, $3)
            on conflict (id) do update
            set
              data = excluded.data,
              expiry_date = excluded.expiry_date
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        conn.execute(
            &query,
            &[
                &record.id.to_string(),
                &rmp_serde::to_vec(&record).map_err(Error::Encode)?,
                &record.expiry_date,
            ],
        )
        .await?;

        Ok(())
    }
}

#[async_trait]
impl ExpiredDeletion for PostgresStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        let query = format!(
            r#"
            delete from "{schema_name}"."{table_name}"
            where expiry_date < (now() at time zone 'utc')
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        let client = self.pool.get().await.map_err(Error::Pool)?;
        client.execute(&query, &[]).await.map_err(Error::Pg)?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for PostgresStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        let mut client = self.pool.get().await.map_err(Error::Pool)?;
        let tx = client.transaction().await.map_err(Error::Pg)?;

        while self.id_exists(&tx, &record.id).await? {
            record.id = Id::default();
        }

        self.save_with_conn(&tx, record).await?;
        tx.commit().await.map_err(Error::Pg)?;
        Ok(())
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let mut client = self.pool.get().await.map_err(Error::Pool)?;
        let tx = client.transaction().await.map_err(Error::Pg)?;
        self.save_with_conn(&tx, record).await?;
        tx.commit().await.map_err(Error::Pg)?;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let query = format!(
            r#"
            select data from "{schema_name}"."{table_name}"
            where id = $1 and expiry_date > $2
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        let client = self.pool.get().await.map_err(Error::Pool)?;
        let record_value: Option<Vec<u8>> = client
            .query_opt(
                &query,
                &[&session_id.to_string(), &OffsetDateTime::now_utc()],
            )
            .await
            .map_err(Error::Pg)?
            .map(|row| row.get(0));

        if let Some(data) = record_value {
            Ok(Some(rmp_serde::from_slice(&data).map_err(Error::Decode)?))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        let query = format!(
            r#"delete from "{schema_name}"."{table_name}" where id = $1"#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        let client = self.pool.get().await.map_err(Error::Pool)?;
        client
            .execute(&query, &[&session_id.to_string()])
            .await
            .map_err(Error::Pg)?;

        Ok(())
    }
}

/// A valid PostreSQL identifier must start with a letter or underscore
/// (including letters with diacritical marks and non-Latin letters). Subsequent
/// characters in an identifier or key word can be letters, underscores, digits
/// (0-9), or dollar signs ($). See https://www.postgresql.org/docs/current/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS for details.
fn is_valid_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .map(|c| c.is_alphabetic() || c == '_')
            .unwrap_or_default()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}
