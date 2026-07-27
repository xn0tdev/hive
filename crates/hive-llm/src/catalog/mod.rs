//! Provider model listing + [models.dev](https://models.dev) enrichment.
//!
//! Chat streaming stays in `provider/`; discovery and capabilities live here.

mod enrich;
mod http;
mod models_dev;
mod provider_list;
mod provider_meta;
mod suggest;

pub use enrich::{enrich_models, ModelCard};
pub use models_dev::{fetch_models_dev, warm_cache, ModelsDevCatalog};
pub use provider_list::{list_provider_models, RemoteModel};
pub use provider_meta::{models_dev_hint_for_base, provider_label_for_base};
pub use suggest::{suggest_model, suggest_roles, RolePicks};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("http: {0}")]
    Http(String),
    #[error("api: {0}")]
    Api(String),
    #[error("parse: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, CatalogError>;
