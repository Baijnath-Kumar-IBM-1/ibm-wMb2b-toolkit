//! Server-sent events. The client reads the stream with `fetch` so it can
//! send the bearer token in a header (`EventSource` cannot).

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::get;
use axum::{Router, extract::State};
use futures_util::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::Shared;

pub fn router() -> Router<Shared> {
    Router::new().route("/events", get(stream))
}

async fn stream(
    State(state): State<Shared>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let mut shutdown = state.shutdown.subscribe();
    let events = BroadcastStream::new(state.events.subscribe())
        .map(|item| {
            Ok(match item {
                Ok(event) => SseEvent::default()
                    .event("event")
                    .json_data(&event)
                    .unwrap_or_else(|_| SseEvent::default().event("error").data("encoding")),
                // A slow reader missed events; the client can reload state.
                Err(BroadcastStreamRecvError::Lagged(missed)) => {
                    SseEvent::default().event("lagged").data(missed.to_string())
                }
            })
        })
        .take_until(async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        });
    Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
