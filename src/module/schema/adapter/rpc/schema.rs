use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::module::schema::adapter::rpc::mapper;
use crate::module::schema::core::ports::{GenerateRequest, SchemaService, ValidationTarget};
use crate::package::errdef::Error;
use crate::package::pagination::ListRequest;
use crate::rpc::v1;

/// The transport adapter. It translates, delegates, and translates back. No
/// schema meaning lives here, which is what keeps the core reusable behind a
/// second transport.
pub struct SchemaHandler {
    service: Arc<dyn SchemaService>,
}

impl SchemaHandler {
    pub fn new(service: Arc<dyn SchemaService>) -> Self {
        Self { service }
    }

    fn target_from(
        target: Option<v1::validate_schema_request::Target>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::validate_schema_request::Target::Id(id)) => Ok(ValidationTarget::Stored(id)),
            Some(v1::validate_schema_request::Target::Draft(draft)) => Ok(ValidationTarget::Draft(
                Box::new(mapper::draft_schema_from(draft)?),
            )),
            None => Err(Error::validation("failed payload validation")
                .add_violation("target", "field is required: name either an id or a draft")),
        }
    }

    fn generate_target_from(
        target: Option<v1::generate_ddl_request::Target>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::generate_ddl_request::Target::Id(id)) => Ok(ValidationTarget::Stored(id)),
            Some(v1::generate_ddl_request::Target::Draft(draft)) => Ok(ValidationTarget::Draft(
                Box::new(mapper::draft_schema_from(draft)?),
            )),
            None => Err(Error::validation("failed payload validation")
                .add_violation("target", "field is required: name either an id or a draft")),
        }
    }
}

#[tonic::async_trait]
impl v1::schema_service_server::SchemaService for SchemaHandler {
    #[tracing::instrument(name = "SchemaService.CreateSchema", skip_all)]
    async fn create_schema(
        &self,
        request: Request<v1::CreateSchemaRequest>,
    ) -> Result<Response<v1::CreateSchemaResponse>, Status> {
        let request = request.into_inner();

        let draft = mapper::draft_from(
            request.name,
            request.description,
            request.entities,
            request.relationships,
        )?;

        let schema = self.service.create(draft).await?;

        Ok(Response::new(v1::CreateSchemaResponse {
            schema: Some(mapper::schema_to(schema)),
        }))
    }

    #[tracing::instrument(name = "SchemaService.GetSchema", skip_all)]
    async fn get_schema(
        &self,
        request: Request<v1::GetSchemaRequest>,
    ) -> Result<Response<v1::GetSchemaResponse>, Status> {
        let schema = self.service.find_by_id(&request.into_inner().id).await?;

        Ok(Response::new(v1::GetSchemaResponse {
            schema: Some(mapper::schema_to(schema)),
        }))
    }

    #[tracing::instrument(name = "SchemaService.ListSchemas", skip_all)]
    async fn list_schemas(
        &self,
        request: Request<v1::ListSchemasRequest>,
    ) -> Result<Response<v1::ListSchemasResponse>, Status> {
        let request = request.into_inner();

        let page = self
            .service
            .index(ListRequest::paged(
                request.page.into(),
                request.per_page.into(),
            ))
            .await?;

        Ok(Response::new(v1::ListSchemasResponse {
            schemas: page.data.into_iter().map(mapper::summary_to).collect(),
            page: page.meta.current_page as u32,
            per_page: page.meta.per_page as u32,
            total: page.meta.total_count as u64,
        }))
    }

    #[tracing::instrument(name = "SchemaService.UpdateSchema", skip_all)]
    async fn update_schema(
        &self,
        request: Request<v1::UpdateSchemaRequest>,
    ) -> Result<Response<v1::UpdateSchemaResponse>, Status> {
        let request = request.into_inner();

        let draft = mapper::draft_from(
            request.name,
            request.description,
            request.entities,
            request.relationships,
        )?;

        let schema = self.service.replace(&request.id, draft).await?;

        Ok(Response::new(v1::UpdateSchemaResponse {
            schema: Some(mapper::schema_to(schema)),
        }))
    }

    #[tracing::instrument(name = "SchemaService.DeleteSchema", skip_all)]
    async fn delete_schema(
        &self,
        request: Request<v1::DeleteSchemaRequest>,
    ) -> Result<Response<v1::DeleteSchemaResponse>, Status> {
        self.service.delete(&request.into_inner().id).await?;

        Ok(Response::new(v1::DeleteSchemaResponse {}))
    }

    #[tracing::instrument(name = "SchemaService.ValidateSchema", skip_all)]
    async fn validate_schema(
        &self,
        request: Request<v1::ValidateSchemaRequest>,
    ) -> Result<Response<v1::ValidateSchemaResponse>, Status> {
        let target = Self::target_from(request.into_inner().target)?;
        let report = self.service.validate(target).await?;

        Ok(Response::new(v1::ValidateSchemaResponse {
            valid: report.is_valid(),
            diagnostics: mapper::report_to(report),
        }))
    }

    #[tracing::instrument(name = "SchemaService.GenerateDdl", skip_all)]
    async fn generate_ddl(
        &self,
        request: Request<v1::GenerateDdlRequest>,
    ) -> Result<Response<v1::GenerateDdlResponse>, Status> {
        let request = request.into_inner();

        let generated = self
            .service
            .generate_ddl(GenerateRequest {
                target: Self::generate_target_from(request.target)?,
                dialect: mapper::dialect_from(request.dialect),
                include_comments: request.include_comments,
            })
            .await?;

        Ok(Response::new(v1::GenerateDdlResponse {
            ddl: generated.ddl,
            diagnostics: mapper::report_to(generated.report),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use tonic::Code;

    use crate::module::schema::core::domain::{Report, Schema, SchemaDraft, SchemaSummary};
    use crate::module::schema::core::ports::Generated;
    use crate::package::pagination::{Data, DEFAULT_PER_PAGE};
    use v1::schema_service_server::SchemaService as _;

    /// Records what the handler passed down, so a test can assert the adapter
    /// translated rather than that the core behaved.
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

        fn schema(&self) -> Schema {
            Schema::new(
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
        async fn index(&self, request: ListRequest) -> Result<Data<SchemaSummary>, Error> {
            self.guard()?;

            Ok(Data::new(
                vec![self.schema().summary()],
                1,
                request.page,
                request.per_page,
            ))
        }

        async fn create(&self, draft: SchemaDraft) -> Result<Schema, Error> {
            self.guard()?;

            Ok(Schema::new("s1", draft, chrono::Utc::now()))
        }

        async fn find_by_id(&self, _id: &str) -> Result<Schema, Error> {
            self.guard()?;

            Ok(self.schema())
        }

        async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, Error> {
            self.guard()?;

            Ok(Schema::new(id, draft, chrono::Utc::now()))
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

    fn handler(service: SpyService) -> SchemaHandler {
        SchemaHandler::new(Arc::new(service))
    }

    #[tokio::test]
    async fn a_create_returns_the_stored_schema() {
        let handler = handler(SpyService::default());

        let response = handler
            .create_schema(Request::new(v1::CreateSchemaRequest {
                name: "blog".to_owned(),
                description: String::new(),
                entities: vec![],
                relationships: vec![],
            }))
            .await
            .expect("the call should succeed")
            .into_inner();

        assert_eq!(
            response.schema.map(|schema| schema.name),
            Some("blog".to_owned())
        );
    }

    #[tokio::test]
    async fn a_malformed_request_never_reaches_the_service() {
        let handler = handler(SpyService::failing(|| {
            Error::unknown("the service should never be reached")
        }));

        let status = handler
            .create_schema(Request::new(v1::CreateSchemaRequest {
                name: "blog".to_owned(),
                description: String::new(),
                entities: vec![v1::Entity {
                    id: String::new(),
                    name: String::new(),
                    description: String::new(),
                    attributes: vec![],
                    position: None,
                }],
                relationships: vec![],
            }))
            .await
            .expect_err("the call should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn a_listing_carries_the_page_and_the_total() {
        let handler = handler(SpyService::default());

        let response = handler
            .list_schemas(Request::new(v1::ListSchemasRequest {
                page: 1,
                per_page: 10,
            }))
            .await
            .expect("the call should succeed")
            .into_inner();

        assert_eq!(response.schemas.len(), 1);
        assert_eq!(response.total, 1);
        assert_eq!(response.page, 1);
        assert_eq!(response.per_page, 10);
    }

    #[tokio::test]
    async fn an_out_of_range_page_request_is_clamped_before_it_is_answered() {
        let handler = handler(SpyService::default());

        let response = handler
            .list_schemas(Request::new(v1::ListSchemasRequest {
                page: 0,
                per_page: 0,
            }))
            .await
            .expect("the call should succeed")
            .into_inner();

        assert_eq!(response.page, 1, "page 0 is not a page");
        assert_eq!(
            i64::from(response.per_page),
            DEFAULT_PER_PAGE,
            "an unset size takes the default"
        );
    }

    #[tokio::test]
    async fn a_not_found_becomes_the_matching_transport_code() {
        let handler = handler(SpyService::failing(|| Error::not_found("nope")));

        let status = handler
            .get_schema(Request::new(v1::GetSchemaRequest {
                id: "missing".to_owned(),
            }))
            .await
            .expect_err("the call should fail");

        assert_eq!(status.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn a_validate_without_a_target_is_rejected() {
        let handler = handler(SpyService::failing(|| {
            Error::unknown("the service should never be reached")
        }));

        let status = handler
            .validate_schema(Request::new(v1::ValidateSchemaRequest { target: None }))
            .await
            .expect_err("the call should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn an_unimplemented_rpc_says_so_rather_than_returning_an_empty_result() {
        let handler = handler(SpyService::failing(|| {
            Error::unimplemented("ValidateSchema lands in milestone M2")
        }));

        let status = handler
            .validate_schema(Request::new(v1::ValidateSchemaRequest {
                target: Some(v1::validate_schema_request::Target::Id("s1".to_owned())),
            }))
            .await
            .expect_err("the call should fail");

        assert_eq!(
            status.code(),
            Code::Unimplemented,
            "a caller must not mistake a stub for a clean bill of health"
        );
    }

    #[tokio::test]
    async fn a_delete_returns_an_empty_response_rather_than_the_deleted_schema() {
        let handler = handler(SpyService::default());

        handler
            .delete_schema(Request::new(v1::DeleteSchemaRequest {
                id: "s1".to_owned(),
            }))
            .await
            .expect("the call should succeed");
    }
}
