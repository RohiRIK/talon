//! `GET /api/v1/events` — Server-Sent Events feed of [`RunEvent`]s.
//!
//! Auth note: browsers' `EventSource` cannot set an `Authorization` header,
//! so this endpoint also accepts `?token=` (validated by the same middleware
//! as the rest of the API). The daemon never logs request URIs on this path,
//! so the token does not leak into logs.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use futures::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use super::WebState;

pub async fn events(
    State(state): State<WebState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => serde_json::to_string(&ev)
                .ok()
                .map(|data| Ok(Event::default().data(data))),
            // Lagged: this subscriber was too slow and missed events — the
            // console reconciles by refetching, so just skip the gap marker.
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
