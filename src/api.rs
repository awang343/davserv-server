use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;

use crate::dav::CardDavClient;
use crate::vcard::{self, Contact, ContactInput};

#[derive(Clone)]
pub struct AppState {
    pub dav: Arc<CardDavClient>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/contacts", get(list_contacts).post(create_contact))
        .route(
            "/api/contacts/:uid",
            get(get_contact).put(update_contact).delete(delete_contact),
        )
        .with_state(state)
}

pub enum AppError {
    NotFound,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => {
                (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
            }
            AppError::Internal(e) => {
                tracing::error!("internal error: {e:#}");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": format!("{e}")})),
                )
                    .into_response()
            }
        }
    }
}

type ApiResult<T> = Result<T, AppError>;

async fn list_contacts(State(state): State<AppState>) -> ApiResult<Json<Vec<Contact>>> {
    let cards = state.dav.list().await?;
    let mut contacts = Vec::with_capacity(cards.len());
    for card in cards {
        let vc = vcard::parse(&card.vcard_text)?;
        contacts.push(vcard::contact_from_vcard(&card.uid, &vc));
    }
    contacts.sort_by(|a, b| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()));
    Ok(Json(contacts))
}

async fn get_contact(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> ApiResult<Json<Contact>> {
    let card = state.dav.get(&uid).await?.ok_or(AppError::NotFound)?;
    let vc = vcard::parse(&card.vcard_text)?;
    Ok(Json(vcard::contact_from_vcard(&card.uid, &vc)))
}

async fn create_contact(
    State(state): State<AppState>,
    Json(input): Json<ContactInput>,
) -> ApiResult<(StatusCode, Json<Contact>)> {
    let uid = uuid::Uuid::new_v4().to_string();
    let vcard_text = vcard::vcard_from_input(&uid, &input);
    state.dav.create(&uid, &vcard_text).await?;
    let vc = vcard::parse(&vcard_text)?;
    let contact = vcard::contact_from_vcard(&uid, &vc);
    Ok((StatusCode::CREATED, Json(contact)))
}

async fn update_contact(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(input): Json<ContactInput>,
) -> ApiResult<Json<Contact>> {
    let existing = state.dav.get(&uid).await?.ok_or(AppError::NotFound)?;
    let vcard_text = vcard::vcard_from_input(&uid, &input);
    state
        .dav
        .update(&uid, &vcard_text, existing.etag.as_deref())
        .await?;
    let vc = vcard::parse(&vcard_text)?;
    Ok(Json(vcard::contact_from_vcard(&uid, &vc)))
}

async fn delete_contact(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> ApiResult<StatusCode> {
    let deleted = state.dav.delete(&uid, None).await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}
