//! What each model takes, as the server says.

use async_trait::async_trait;

use crate::models::ModelConstraintTable;
use crate::ports::PortError;

/// A provider that can say what each model takes, from the server rather than the app's constants. Slow and
/// rarely changing, so the app caches the answer and asks again at most once a day.
#[async_trait]
pub trait ModelConstraintsSource: Send + Sync {
    /// The server's constraints of the models the provider runs; `None` when it has no way to tell.
    async fn model_constraints(&self) -> Result<Option<ModelConstraintTable>, PortError>;
}
