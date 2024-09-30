use axum::{routing::get, Router};
use http::{header, HeaderMap};
use http_body_util::BodyExt;
use time::Duration;
use tower_cookies::{cookie, Cookie};
use tower_sessions::{Expiry, Session, SessionManagerLayer, SessionStore};

use axum::body::Body;
use tower_sessions_postgres_store::PostgresStore;

fn routes() -> Router {
    Router::new()
        .route("/", get(|_: Session| async move { "Hello, world!" }))
        .route(
            "/insert",
            get(|session: Session| async move {
                session.insert("foo", 42).await.unwrap();
            }),
        )
        .route(
            "/get",
            get(|session: Session| async move {
                format!("{}", session.get::<usize>("foo").await.unwrap().unwrap())
            }),
        )
        .route(
            "/get_value",
            get(|session: Session| async move {
                format!("{:?}", session.get_value("foo").await.unwrap())
            }),
        )
        .route(
            "/remove",
            get(|session: Session| async move {
                session.remove::<usize>("foo").await.unwrap();
            }),
        )
        .route(
            "/remove_value",
            get(|session: Session| async move {
                session.remove_value("foo").await.unwrap();
            }),
        )
        .route(
            "/cycle_id",
            get(|session: Session| async move {
                session.cycle_id().await.unwrap();
            }),
        )
        .route(
            "/flush",
            get(|session: Session| async move {
                session.flush().await.unwrap();
            }),
        )
}

pub fn build_app<Store: SessionStore + Clone>(
    mut session_manager: SessionManagerLayer<Store>,
    max_age: Option<Duration>,
) -> Router {
    if let Some(max_age) = max_age {
        session_manager = session_manager.with_expiry(Expiry::OnInactivity(max_age));
    }

    routes().layer(session_manager)
}

pub async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into()
}

pub async fn create_app(max_age: Option<Duration>) -> Router {
    let database_url = std::option_env!("DATABASE_URL").expect("DATABASE_URL must be set");
    let manager =
        deadpool_postgres::Manager::new(database_url.parse().unwrap(), tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager).build().unwrap();
    let session_store = PostgresStore::new(pool);
    session_store.migrate().await.unwrap();
    let session_manager = SessionManagerLayer::new(session_store).with_secure(true);

    build_app(session_manager, max_age)
}

pub fn get_session_cookie(headers: &HeaderMap) -> Result<Cookie<'_>, cookie::ParseError> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .flat_map(|header| header.to_str())
        .next()
        .ok_or(cookie::ParseError::MissingPair)
        .and_then(Cookie::parse_encoded)
}
