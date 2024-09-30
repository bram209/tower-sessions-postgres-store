## Overview
This is a `SessionStore` for the [`tower-sessions`](https://github.com/maxcountryman/tower-sessions) middleware which uses [tokio-postgres](https://github.com/sfackler/rust-postgres) for handling Postgres databases.
It is directly based on the [`sqlx-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/sqlx-store) and inherited the test suite.

## Usage

See the [counter example](./examples/counter.rs) for a complete example.

```rust
pub fn create_pool(database_url: &str) -> Pool {
    let config = database_url.parse().unwrap();
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(manager).build().unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // create the session store and run it's migration
    let database_url = std::option_env!("DATABASE_URL").expect("Missing DATABASE_URL.");
    let pool = create_pool(database_url);
    let session_store = PostgresStore::new(pool);
    session_store.migrate().await?;

    // create the session layer
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(10)));

    // wire it up with axum...
    let app = Router::new().route("/", get(handler)).layer(session_layer);
    // ...
}
```

and use it as such:

```rust
const COUNTER_KEY: &str = "counter";

#[derive(Serialize, Deserialize, Default)]
struct Counter(usize);

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session.get(COUNTER_KEY).await.unwrap().unwrap_or_default();
    session.insert(COUNTER_KEY, counter.0 + 1).await.unwrap();
    format!("Current count: {}", counter.0)
}
```
