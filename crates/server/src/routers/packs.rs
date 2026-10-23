use axum::routing::get;
use axum::{Json, Router, extract::State};
use serde::Serialize;
use trace_core::packs::Sender;

use super::Shared;

pub fn router() -> Router<Shared> {
    Router::new().route("/packs", get(list))
}

#[derive(Serialize)]
struct PackView {
    id: String,
    label: String,
    standard: String,
    version: String,
    documents: Vec<PackDocument>,
}

#[derive(Serialize)]
struct PackDocument {
    code: String,
    name: String,
    note: String,
    /// `p1` or `p2`.
    sender: &'static str,
}

async fn list(State(state): State<Shared>) -> Json<Vec<PackView>> {
    Json(
        state
            .catalog
            .all()
            .iter()
            .map(|pack| {
                let meta = &pack.manifest.pack;
                PackView {
                    id: meta.id.clone(),
                    label: meta.label.clone(),
                    standard: meta.standard.clone(),
                    version: meta.version.clone(),
                    documents: pack
                        .documents()
                        .iter()
                        .map(|d| PackDocument {
                            code: d.code.clone(),
                            name: d.name.clone(),
                            note: d.note.clone(),
                            sender: match d.sender {
                                Sender::P1 => "p1",
                                Sender::P2 => "p2",
                            },
                        })
                        .collect(),
                }
            })
            .collect(),
    )
}
