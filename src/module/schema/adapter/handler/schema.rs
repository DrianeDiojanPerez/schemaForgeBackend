use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json as AxumJson;
use serde::{Deserialize, Serialize};

use crate::module::schema::adapter::handler::mapper::{
    self, Diagnostic, Schema, SchemaPayload, SchemaSummary, Violations,
};
use crate::module::schema::core::ports::{GenerateRequest, SchemaService, ValidationTarget};
use crate::package::errdef::Error;
use crate::package::extract::Json;
use crate::package::pagination::ListRequest;
use crate::package::response::{self, Response};

pub type SchemaState = Arc<dyn SchemaService>;

#[derive(Debug, Deserialize)]
pub struct ValidateRequest {
    #[serde(default)]
    pub schema_id: Option<String>,
    #[serde(default)]
    pub schema: Option<SchemaPayload>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateDdlRequest {
    #[serde(default)]
    pub schema_id: Option<String>,
    #[serde(default)]
    pub schema: Option<SchemaPayload>,
    #[serde(default)]
    pub dialect: Option<String>,
    #[serde(default)]
    pub include_comments: bool,
}

#[derive(Debug, Serialize)]
pub struct Validation {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
pub struct GeneratedDdl {
    pub ddl: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
pub struct DeletedSchema {
    pub message: &'static str,
    pub schema_id: String,
}

/// Validation and generation both run against something already stored or
/// against a diagram the canvas is still holding, so a request names one or
/// the other rather than forcing a caller to save a half-drawn schema.
fn target_from(
    schema_id: Option<String>,
    schema: Option<SchemaPayload>,
) -> Result<ValidationTarget, Error> {
    match (schema_id, schema) {
        (Some(id), None) => Ok(ValidationTarget::Stored(id)),
        (None, Some(payload)) => Ok(ValidationTarget::Draft(Box::new(
            mapper::draft_schema_from(payload)?,
        ))),
        (Some(_), Some(_)) => Err(
            Error::validation("failed payload validation").add_violation(
                "schema_id",
                "field cannot be given alongside `schema`: name one target",
            ),
        ),
        (None, None) => Err(
            Error::validation("failed payload validation").add_violation(
                "schema_id",
                "field is required: name either a stored id or a schema to check",
            ),
        ),
    }
}

#[tracing::instrument(name = "SchemaHandler.Index", skip_all)]
pub async fn index(
    State(service): State<SchemaState>,
    request: ListRequest,
) -> Result<AxumJson<Response<Vec<SchemaSummary>>>, Error> {
    let page = service.index(request).await?.map(SchemaSummary::from);

    Ok(response::ok_paginated(page.data, page.meta))
}

#[tracing::instrument(name = "SchemaHandler.Create", skip_all)]
pub async fn create(
    State(service): State<SchemaState>,
    Json(payload): Json<SchemaPayload>,
) -> Result<AxumJson<Response<Schema>>, Error> {
    let schema = service.create(mapper::draft_from(payload)?).await?;

    Ok(response::ok(Schema::from(schema)))
}

#[tracing::instrument(name = "SchemaHandler.Get", skip_all)]
pub async fn get(
    State(service): State<SchemaState>,
    Path(schema_id): Path<String>,
) -> Result<AxumJson<Response<Schema>>, Error> {
    let schema = service.find_by_id(&schema_id).await?;

    Ok(response::ok(Schema::from(schema)))
}

/// The canvas holds the authoritative picture, so it sends the whole picture
/// rather than a diff the backend would have to reassemble.
#[tracing::instrument(name = "SchemaHandler.Replace", skip_all)]
pub async fn replace(
    State(service): State<SchemaState>,
    Path(schema_id): Path<String>,
    Json(payload): Json<SchemaPayload>,
) -> Result<AxumJson<Response<Schema>>, Error> {
    let schema = service
        .replace(&schema_id, mapper::draft_from(payload)?)
        .await?;

    Ok(response::ok(Schema::from(schema)))
}

#[tracing::instrument(name = "SchemaHandler.Delete", skip_all)]
pub async fn delete(
    State(service): State<SchemaState>,
    Path(schema_id): Path<String>,
) -> Result<AxumJson<Response<DeletedSchema>>, Error> {
    service.delete(&schema_id).await?;

    Ok(response::ok(DeletedSchema {
        message: "Schema deleted successfully",
        schema_id,
    }))
}

#[tracing::instrument(name = "SchemaHandler.Validate", skip_all)]
pub async fn validate(
    State(service): State<SchemaState>,
    Json(payload): Json<ValidateRequest>,
) -> Result<AxumJson<Response<Validation>>, Error> {
    let target = target_from(payload.schema_id, payload.schema)?;
    let report = service.validate(target).await?;

    Ok(response::ok(Validation {
        valid: report.is_valid(),
        diagnostics: mapper::report_to(report),
    }))
}

#[tracing::instrument(name = "SchemaHandler.GenerateDdl", skip_all)]
pub async fn generate_ddl(
    State(service): State<SchemaState>,
    Json(payload): Json<GenerateDdlRequest>,
) -> Result<AxumJson<Response<GeneratedDdl>>, Error> {
    let mut violations = Violations::default();
    let dialect = mapper::dialect_from(payload.dialect.as_deref(), "dialect", &mut violations);

    violations.into_result()?;

    let generated = service
        .generate_ddl(GenerateRequest {
            target: target_from(payload.schema_id, payload.schema)?,
            dialect: dialect.unwrap_or_default(),
            include_comments: payload.include_comments,
        })
        .await?;

    Ok(response::ok(GeneratedDdl {
        ddl: generated.ddl,
        diagnostics: mapper::report_to(generated.report),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;

    use crate::module::schema::core::domain::{
        self as domain, Report, SchemaDraft, SchemaSummary as DomainSummary,
    };
    use crate::module::schema::core::ports::Generated;
    use crate::package::errdef::code;
    use crate::package::pagination::Data;

    #[derive(Default)]
    struct SpyService {
        fail_with: Option<fn() -> Error>,
    }

    impl SpyService {
        fn failing(failure: fn() -> Error) -> Self {
            Self {
                fail_with: Some(failure),
            }
        }

        fn guard(&self) -> Result<(), Error> {
            match self.fail_with {
                Some(failure) => Err(failure()),
                None => Ok(()),
            }
        }

        fn schema(&self) -> domain::Schema {
            domain::Schema::new(
                "s1",
                SchemaDraft {
                    name: "blog".to_owned(),
                    ..SchemaDraft::default()
                },
                chrono::Utc::now(),
            )
        }
    }

    #[async_trait]
    impl SchemaService for SpyService {
        async fn index(&self, request: ListRequest) -> Result<Data<DomainSummary>, Error> {
            self.guard()?;

            Ok(Data::new(
                vec![self.schema().summary()],
                1,
                request.page,
                request.per_page,
            ))
        }

        async fn create(&self, draft: SchemaDraft) -> Result<domain::Schema, Error> {
            self.guard()?;

            Ok(domain::Schema::new("s1", draft, chrono::Utc::now()))
        }

        async fn find_by_id(&self, _id: &str) -> Result<domain::Schema, Error> {
            self.guard()?;

            Ok(self.schema())
        }

        async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<domain::Schema, Error> {
            self.guard()?;

            Ok(domain::Schema::new(id, draft, chrono::Utc::now()))
        }

        async fn delete(&self, _id: &str) -> Result<(), Error> {
            self.guard()
        }

        async fn validate(&self, _target: ValidationTarget) -> Result<Report, Error> {
            self.guard()?;

            Ok(Report::default())
        }

        async fn generate_ddl(&self, _request: GenerateRequest) -> Result<Generated, Error> {
            self.guard()?;

            Ok(Generated {
                ddl: "CREATE TABLE users ();".to_owned(),
                report: Report::default(),
            })
        }
    }

    fn state(service: SpyService) -> State<SchemaState> {
        State(Arc::new(service))
    }

    fn payload(name: &str) -> SchemaPayload {
        serde_json::from_value(serde_json::json!({ "name": name }))
            .expect("the payload should parse")
    }

    #[tokio::test]
    async fn a_create_returns_the_stored_schema() {
        let response = create(state(SpyService::default()), Json(payload("blog")))
            .await
            .expect("the call should succeed");

        assert_eq!(
            response.0.data.map(|schema| schema.name),
            Some("blog".to_owned())
        );
    }

    #[tokio::test]
    async fn a_malformed_request_never_reaches_the_service() {
        let malformed = serde_json::from_value(serde_json::json!({
            "name": "blog",
            "entities": [{ "id": "", "name": "" }],
        }))
        .expect("the payload should parse");

        let error = create(
            state(SpyService::failing(|| {
                Error::unknown("the service should never be reached")
            })),
            Json(malformed),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn a_listing_carries_the_page_and_the_total() {
        let response = index(
            state(SpyService::default()),
            ListRequest::from_query("page=1&per_page=10"),
        )
        .await
        .expect("the call should succeed");

        let body = response.0;

        assert_eq!(body.data.map(|schemas| schemas.len()), Some(1));
        assert_eq!(
            body.pagination.map(|meta| meta["total_count"].clone()),
            Some(serde_json::json!(1))
        );
    }

    #[tokio::test]
    async fn an_out_of_range_page_request_is_clamped_before_it_is_answered() {
        let response = index(
            state(SpyService::default()),
            ListRequest::from_query("page=0&per_page=0"),
        )
        .await
        .expect("the call should succeed");

        let pagination = response.0.pagination.expect("a listing is paginated");

        assert_eq!(pagination["current_page"], 1, "page 0 is not a page");
        assert_eq!(
            pagination["per_page"], 10,
            "an unset size takes the default"
        );
    }

    #[tokio::test]
    async fn a_not_found_reaches_the_caller_as_a_not_found() {
        let error = get(
            state(SpyService::failing(|| Error::not_found("nope"))),
            Path("missing".to_owned()),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_validate_without_a_target_is_rejected() {
        let error = validate(
            state(SpyService::failing(|| {
                Error::unknown("the service should never be reached")
            })),
            Json(ValidateRequest {
                schema_id: None,
                schema: None,
            }),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn a_validate_naming_two_targets_is_rejected() {
        let error = validate(
            state(SpyService::default()),
            Json(ValidateRequest {
                schema_id: Some("s1".to_owned()),
                schema: Some(payload("blog")),
            }),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn an_unimplemented_endpoint_says_so_rather_than_returning_an_empty_result() {
        let error = validate(
            state(SpyService::failing(|| {
                Error::unimplemented("ValidateSchema lands in milestone M2")
            })),
            Json(ValidateRequest {
                schema_id: Some("s1".to_owned()),
                schema: None,
            }),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(
            error.app_code(),
            code::UNIMPLEMENTED,
            "a caller must not mistake a stub for a clean bill of health"
        );
    }

    #[tokio::test]
    async fn a_dialect_with_no_generator_is_rejected_before_the_service() {
        let error = generate_ddl(
            state(SpyService::failing(|| {
                Error::unknown("the service should never be reached")
            })),
            Json(GenerateDdlRequest {
                schema_id: Some("s1".to_owned()),
                schema: None,
                dialect: Some("sqlite".to_owned()),
                include_comments: false,
            }),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn a_delete_names_what_it_deleted_rather_than_returning_the_schema() {
        let response = delete(state(SpyService::default()), Path("s1".to_owned()))
            .await
            .expect("the call should succeed");

        let deleted = response.0.data.expect("a body should come back");

        assert_eq!(deleted.schema_id, "s1");
        assert_eq!(deleted.message, "Schema deleted successfully");
    }
}
