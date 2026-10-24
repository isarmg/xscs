use super::*;
use crate::admin_core::{AdministratorManagementContext, ManagementError};
pub(super) fn account_routes<Store: AdministratorStore + 'static>(
    router: Router<AdapterState<Store>>,
) -> Router<AdapterState<Store>> {
    router.route(crate::contracts::ADMIN_ACCOUNT_PATH, post(account::<Store>))
}

async fn account<Store: AdministratorStore + 'static>(
    State(state): State<AdapterState<Store>>,
    request: Request,
) -> Response {
    let context = match authorize(&state, request.headers(), request.uri(), request.method()).await
    {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let id = context_id(&context);
    let input = match input::<crate::contracts::AdministratorAccountRequest>(
        request,
        &state.body_admission,
        id.as_ref(),
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return *response,
    };
    mutation_response(
        state
            .service
            .update_own_account(
                &context,
                &input.username,
                &input.current_password,
                input.new_password.as_deref(),
            )
            .await,
        id.as_ref(),
    )
}

async fn authorize<Store: AdministratorStore + 'static>(
    state: &AdapterState<Store>,
    headers: &HeaderMap,
    uri: &Uri,
    method: &Method,
) -> Result<AdministratorManagementContext, Box<Response>> {
    let id = request_id(headers)?;
    let identity = authenticate_request(
        &state.service,
        headers,
        uri,
        method,
        &state.product_id,
        state.mode,
    )
    .await?;
    Ok(AdministratorManagementContext {
        identity,
        request_id: id.map(|id| id.to_string()),
        now_micros: now_micros()?,
    })
}

fn context_id(context: &AdministratorManagementContext) -> Option<RequestId> {
    context
        .request_id
        .as_ref()
        .map(|id| RequestId::new(id.clone()).expect("authorization validated request id"))
}

async fn input<T: serde::de::DeserializeOwned>(
    request: Request,
    admission: &body::BodyAdmission,
    id: Option<&RequestId>,
) -> Result<T, Box<Response>> {
    let bytes = admission
        .read(request, body::Scope::Management, true, id)
        .await?;
    serde_json::from_slice(&bytes).map_err(|_| Box::new(invalid(id)))
}

fn invalid(id: Option<&RequestId>) -> Response {
    error(StatusCode::BAD_REQUEST, "admin.invalid_request", false, id)
}

fn mutation_response<E: std::error::Error + Send + Sync + 'static>(
    result: Result<(), ManagementError<E>>,
    id: Option<&RequestId>,
) -> Response {
    match result {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => management_error(failure, id),
    }
}

fn management_error<E: std::error::Error + Send + Sync + 'static>(
    failure: ManagementError<E>,
    id: Option<&RequestId>,
) -> Response {
    let (status, code, retryable) = match failure {
        ManagementError::Unsupported => (StatusCode::NOT_FOUND, "admin.unsupported", false),
        ManagementError::Unauthorized => (StatusCode::UNAUTHORIZED, "auth.session_required", false),
        ManagementError::Conflict => (StatusCode::CONFLICT, "admin.conflict", false),
        ManagementError::InvalidInput => return invalid(id),
        ManagementError::InvalidCurrentPassword => (
            StatusCode::FORBIDDEN,
            "admin.current_password_invalid",
            false,
        ),
        ManagementError::Busy => (
            StatusCode::SERVICE_UNAVAILABLE,
            "auth.capacity_unavailable",
            true,
        ),
        ManagementError::Store(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "platform.internal",
            false,
        ),
    };
    error(status, code, retryable, id)
}
