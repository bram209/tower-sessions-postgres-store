mod setup;

use time::Duration;
use tower_cookies::Cookie;

use axum::body::Body;
use http::{header, Request, StatusCode};
use tower_cookies::cookie::SameSite;

use tower::util::ServiceExt;

use setup::*;

#[tokio::test]
async fn no_session_set() {
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let res = create_app(Some(Duration::hours(1)))
        .await
        .oneshot(req)
        .await
        .unwrap();

    assert!(res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .is_none());
}

#[tokio::test]
async fn bogus_session_cookie() {
    let session_cookie = Cookie::new("id", "AAAAAAAAAAAAAAAAAAAAAA");
    let req = Request::builder()
        .uri("/insert")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = create_app(Some(Duration::hours(1)))
        .await
        .oneshot(req)
        .await
        .unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_ne!(session_cookie.value(), "AAAAAAAAAAAAAAAAAAAAAA");
}

#[tokio::test]
async fn malformed_session_cookie() {
    let session_cookie = Cookie::new("id", "malformed");
    let req = Request::builder()
        .uri("/")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = create_app(Some(Duration::hours(1)))
        .await
        .oneshot(req)
        .await
        .unwrap();

    let session_cookie = get_session_cookie(res.headers()).unwrap();
    assert_ne!(session_cookie.value(), "malformed");
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn insert_session() {
    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = create_app(Some(Duration::hours(1)))
        .await
        .oneshot(req)
        .await
        .unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    assert_eq!(session_cookie.name(), "id");
    assert_eq!(session_cookie.http_only(), Some(true));
    assert_eq!(session_cookie.same_site(), Some(SameSite::Strict));
    assert!(session_cookie
        .max_age()
        .is_some_and(|dt| dt <= Duration::hours(1)));
    assert_eq!(session_cookie.secure(), Some(true));
    assert_eq!(session_cookie.path(), Some("/"));
}

#[tokio::test]
async fn session_max_age() {
    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = create_app(None).await.oneshot(req).await.unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    assert_eq!(session_cookie.name(), "id");
    assert_eq!(session_cookie.http_only(), Some(true));
    assert_eq!(session_cookie.same_site(), Some(SameSite::Strict));
    assert!(session_cookie.max_age().is_none());
    assert_eq!(session_cookie.secure(), Some(true));
    assert_eq!(session_cookie.path(), Some("/"));
}

#[tokio::test]
async fn get_session() {
    let app = create_app(Some(Duration::hours(1))).await;

    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    let req = Request::builder()
        .uri("/get")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    assert_eq!(body_string(res.into_body()).await, "42");
}

#[tokio::test]
async fn get_no_value() {
    let app = create_app(Some(Duration::hours(1))).await;

    let req = Request::builder()
        .uri("/get_value")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();

    assert_eq!(body_string(res.into_body()).await, "None");
}

#[tokio::test]
async fn expired() {
    let app = create_app(Some(Duration::seconds(1))).await;

    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    // wait 1.1 seconds to be sure
    tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

    let req = Request::builder()
        .uri("/get_value")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();

    assert_eq!(body_string(res.into_body()).await, "None");
}

#[tokio::test]
async fn remove_last_value() {
    let app = create_app(Some(Duration::hours(1))).await;

    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    let req = Request::builder()
        .uri("/remove_value")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let req = Request::builder()
        .uri("/get_value")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();

    assert_eq!(body_string(res.into_body()).await, "None");
}

#[tokio::test]
async fn cycle_session_id() {
    let app = create_app(Some(Duration::hours(1))).await;

    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let first_session_cookie = get_session_cookie(res.headers()).unwrap();

    let req = Request::builder()
        .uri("/cycle_id")
        .header(header::COOKIE, first_session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let second_session_cookie = get_session_cookie(res.headers()).unwrap();

    let req = Request::builder()
        .uri("/get")
        .header(header::COOKIE, second_session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = dbg!(app.oneshot(req).await).unwrap();

    assert_ne!(first_session_cookie.value(), second_session_cookie.value());
    assert_eq!(body_string(res.into_body()).await, "42");
}

#[tokio::test]
async fn flush_session() {
    let app = create_app(Some(Duration::hours(1))).await;

    let req = Request::builder()
        .uri("/insert")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let session_cookie = get_session_cookie(res.headers()).unwrap();

    let req = Request::builder()
        .uri("/flush")
        .header(header::COOKIE, session_cookie.encoded().to_string())
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();

    let session_cookie = get_session_cookie(res.headers()).unwrap();

    assert_eq!(session_cookie.value(), "");
    assert_eq!(session_cookie.max_age(), Some(Duration::ZERO));
}
