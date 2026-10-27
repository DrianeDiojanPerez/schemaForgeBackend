use std::sync::Arc;

use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::module::schema::adapter::rpc::mapper;
use crate::module::schema::core::ports::{
    CompareRequest, ExportRequest, GenerateOptions, GenerateRequest, ImportRequest, SchemaService,
    ValidationTarget,
};
use crate::package::auth::AuthUser;
use crate::package::errdef::Error;
use crate::package::pagination::ListRequest;
use crate::rpc::v1;

pub struct SchemaHandler {
    service: Arc<dyn SchemaService>,
}

impl SchemaHandler {
    pub fn new(service: Arc<dyn SchemaService>) -> Self {
        Self { service }
    }

    /// The guard put the caller here after it read the token, so a request
    /// without one never got past it.
    fn caller<T>(request: &Request<T>) -> Result<Uuid, Status> {
        request
            .extensions()
            .get::<AuthUser>()
            .map(AuthUser::id)
            .ok_or_else(|| Status::from(Error::unauthorized("malformed or missing jwt token")))
    }

    /// Three messages carry the same choice between a stored id and a drawn
    /// draft, and prost writes each one its own enum, so they are unpacked
    /// into this pair before the one rule about them is applied.
    fn target_of(id: Option<String>, draft: Option<v1::Schema>) -> Result<ValidationTarget, Error> {
        match (id, draft) {
            (Some(id), _) => Ok(ValidationTarget::Stored(id)),
            (_, Some(draft)) => Ok(ValidationTarget::Draft(Box::new(
                mapper::draft_schema_from(draft)?,
            ))),
            (None, None) => Err(Error::validation("failed payload validation")
                .add_violation("target", "field is required: name either an id or a draft")),
        }
    }

    fn target_from(
        target: Option<v1::validate_schema_request::Target>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::validate_schema_request::Target::Id(id)) => Self::target_of(Some(id), None),
            Some(v1::validate_schema_request::Target::Draft(draft)) => {
                Self::target_of(None, Some(draft))
            }
            None => Self::target_of(None, None),
        }
    }

    fn generate_target_from(
        target: Option<v1::generate_ddl_request::Target>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::generate_ddl_request::Target::Id(id)) => Self::target_of(Some(id), None),
            Some(v1::generate_ddl_request::Target::Draft(draft)) => {
                Self::target_of(None, Some(draft))
            }
            None => Self::target_of(None, None),
        }
    }

    fn expected_target_from(
        target: Option<v1::compare_schemas_request::Expected>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::compare_schemas_request::Expected::ExpectedId(id)) => {
                Self::target_of(Some(id), None)
            }
            Some(v1::compare_schemas_request::Expected::ExpectedDraft(draft)) => {
                Self::target_of(None, Some(draft))
            }
            None => Self::target_of(None, None),
        }
    }

    fn actual_target_from(
        target: Option<v1::compare_schemas_request::Actual>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::compare_schemas_request::Actual::ActualId(id)) => {
                Self::target_of(Some(id), None)
            }
            Some(v1::compare_schemas_request::Actual::ActualDraft(draft)) => {
                Self::target_of(None, Some(draft))
            }
            None => Self::target_of(None, None),
        }
    }

    fn export_target_from(
        target: Option<v1::export_schema_request::Target>,
    ) -> Result<ValidationTarget, Error> {
        match target {
            Some(v1::export_schema_request::Target::Id(id)) => Self::target_of(Some(id), None),
            Some(v1::export_schema_request::Target::Draft(draft)) => {
                Self::target_of(None, Some(draft))
            }
            None => Self::target_of(None, None),
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
        let caller = Self::caller(&request)?;
        let request = request.into_inner();

        let draft = mapper::draft_from(
            request.name,
            request.description,
            request.entities,
            request.relationships,
        )?;

        let schema = self.service.create(caller, draft).await?;

        Ok(Response::new(v1::CreateSchemaResponse {
            schema: Some(mapper::schema_to(schema)),
        }))
    }

    #[tracing::instrument(name = "SchemaService.GetSchema", skip_all)]
    async fn get_schema(
        &self,
        request: Request<v1::GetSchemaRequest>,
    ) -> Result<Response<v1::GetSchemaResponse>, Status> {
        let caller = Self::caller(&request)?;
        let schema = self
            .service
            .find_by_id(caller, &request.into_inner().id)
            .await?;

        Ok(Response::new(v1::GetSchemaResponse {
            schema: Some(mapper::schema_to(schema)),
        }))
    }

    #[tracing::instrument(name = "SchemaService.ListSchemas", skip_all)]
    async fn list_schemas(
        &self,
        request: Request<v1::ListSchemasRequest>,
    ) -> Result<Response<v1::ListSchemasResponse>, Status> {
        let caller = Self::caller(&request)?;
        let request = request.into_inner();

        let page = self
            .service
            .index(
                caller,
                ListRequest::paged(request.page.into(), request.per_page.into()),
            )
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
        let caller = Self::caller(&request)?;
        let request = request.into_inner();

        let draft = mapper::draft_from(
            request.name,
            request.description,
            request.entities,
            request.relationships,
        )?;

        let schema = self.service.replace(caller, &request.id, draft).await?;

        Ok(Response::new(v1::UpdateSchemaResponse {
            schema: Some(mapper::schema_to(schema)),
        }))
    }

    #[tracing::instrument(name = "SchemaService.DeleteSchema", skip_all)]
    async fn delete_schema(
        &self,
        request: Request<v1::DeleteSchemaRequest>,
    ) -> Result<Response<v1::DeleteSchemaResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.service
            .delete(caller, &request.into_inner().id)
            .await?;

        Ok(Response::new(v1::DeleteSchemaResponse {}))
    }

    #[tracing::instrument(name = "SchemaService.ValidateSchema", skip_all)]
    async fn validate_schema(
        &self,
        request: Request<v1::ValidateSchemaRequest>,
    ) -> Result<Response<v1::ValidateSchemaResponse>, Status> {
        let caller = Self::caller(&request)?;
        let target = Self::target_from(request.into_inner().target)?;
        let report = self.service.validate(caller, target).await?;

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
        let caller = Self::caller(&request)?;
        let request = request.into_inner();

        let generated = self
            .service
            .generate_ddl(
                caller,
                GenerateRequest {
                    target: Self::generate_target_from(request.target)?,
                    dialect: mapper::dialect_from(request.dialect),
                    options: GenerateOptions {
                        include_comments: request.include_comments,
                        include_positions: request.include_positions.unwrap_or(true),
                    },
                },
            )
            .await?;

        Ok(Response::new(v1::GenerateDdlResponse {
            ddl: generated.ddl,
            diagnostics: mapper::report_to(generated.report),
        }))
    }

    #[tracing::instrument(name = "SchemaService.ExportSchema", skip_all)]
    async fn export_schema(
        &self,
        request: Request<v1::ExportSchemaRequest>,
    ) -> Result<Response<v1::ExportSchemaResponse>, Status> {
        let caller = Self::caller(&request)?;
        let request = request.into_inner();

        let exported = self
            .service
            .export(
                caller,
                ExportRequest {
                    target: Self::export_target_from(request.target)?,
                    format: mapper::exchange_format_from(request.format),
                    include_positions: request.include_positions.unwrap_or(true),
                },
            )
            .await?;

        Ok(Response::new(v1::ExportSchemaResponse {
            file_name: exported.file_name,
            content_type: exported.content_type,
            content: exported.content,
        }))
    }

    #[tracing::instrument(name = "SchemaService.CompareSchemas", skip_all)]
    async fn compare_schemas(
        &self,
        request: Request<v1::CompareSchemasRequest>,
    ) -> Result<Response<v1::CompareSchemasResponse>, Status> {
        let caller = Self::caller(&request)?;
        let request = request.into_inner();

        let comparison = self
            .service
            .compare(
                caller,
                CompareRequest {
                    expected: Self::expected_target_from(request.expected)?,
                    actual: Self::actual_target_from(request.actual)?,
                    include_positions: request.include_positions.unwrap_or(true),
                },
            )
            .await?;

        let equal = comparison.is_equal();
        let (counts, differences) = mapper::comparison_to(comparison);

        Ok(Response::new(v1::CompareSchemasResponse {
            equal,
            counts,
            differences,
        }))
    }

    #[tracing::instrument(name = "SchemaService.ImportSchema", skip_all)]
    async fn import_schema(
        &self,
        request: Request<v1::ImportSchemaRequest>,
    ) -> Result<Response<v1::ImportSchemaResponse>, Status> {
        let request = request.into_inner();

        let imported = self
            .service
            .import(ImportRequest {
                content: request.content,
                format: mapper::import_format_from(request.format),
                include_positions: request.include_positions.unwrap_or(true),
            })
            .await?;

        Ok(Response::new(v1::ImportSchemaResponse {
            draft: Some(mapper::schema_to(imported.draft)),
            diagnostics: mapper::report_to(imported.report),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use async_trait::async_trait;
    use tonic::Code;

    use crate::module::schema::core::domain::{
        Comparison, Report, Schema, SchemaDraft, SchemaSummary,
    };
    use crate::module::schema::core::ports::{Exported, Generated, Imported};
    use crate::package::auth::Identity;
    use crate::package::pagination::{Data, DEFAULT_PER_PAGE};
    use v1::schema_service_server::SchemaService as _;

    /// Records who the handler said was calling, so a test can assert the
    /// adapter handed the caller down rather than that the core behaved.
    #[derive(Default)]
    struct SpyService {
        fail_with: Option<fn() -> Error>,
        callers: Mutex<Vec<Uuid>>,
    }

    impl SpyService {
        fn failing(failure: fn() -> Error) -> Self {
            Self {
                fail_with: Some(failure),
                callers: Mutex::default(),
            }
        }

        fn guard(&self, caller: Uuid) -> Result<(), Error> {
            self.callers
                .lock()
                .expect("the lock should be held")
                .push(caller);

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
        async fn index(
            &self,
            caller: Uuid,
            request: ListRequest,
        ) -> Result<Data<SchemaSummary>, Error> {
            self.guard(caller)?;

            Ok(Data::new(
                vec![self.schema().summary()],
                1,
                request.page,
                request.per_page,
            ))
        }

        async fn create(&self, caller: Uuid, draft: SchemaDraft) -> Result<Schema, Error> {
            self.guard(caller)?;

            Ok(Schema::new("s1", draft, chrono::Utc::now()))
        }

        async fn find_by_id(&self, caller: Uuid, _id: &str) -> Result<Schema, Error> {
            self.guard(caller)?;

            Ok(self.schema())
        }

        async fn replace(
            &self,
            caller: Uuid,
            id: &str,
            draft: SchemaDraft,
        ) -> Result<Schema, Error> {
            self.guard(caller)?;

            Ok(Schema::new(id, draft, chrono::Utc::now()))
        }

        async fn delete(&self, caller: Uuid, _id: &str) -> Result<(), Error> {
            self.guard(caller)
        }

        async fn validate(&self, caller: Uuid, _target: ValidationTarget) -> Result<Report, Error> {
            self.guard(caller)?;

            Ok(Report::default())
        }

        async fn generate_ddl(
            &self,
            caller: Uuid,
            _request: GenerateRequest,
        ) -> Result<Generated, Error> {
            self.guard(caller)?;

            Ok(Generated {
                ddl: "CREATE TABLE users ();".to_owned(),
                report: Report::default(),
            })
        }

        async fn export(&self, caller: Uuid, _request: ExportRequest) -> Result<Exported, Error> {
            self.guard(caller)?;

            Ok(Exported {
                file_name: "blog.json".to_owned(),
                content_type: "application/json".to_owned(),
                content: "{}".to_owned(),
            })
        }

        async fn import(&self, _request: ImportRequest) -> Result<Imported, Error> {
            if let Some(failure) = self.fail_with {
                return Err(failure());
            }

            Ok(Imported {
                draft: self.schema(),
                report: Report::default(),
            })
        }

        async fn compare(
            &self,
            _caller: Uuid,
            _request: CompareRequest,
        ) -> Result<Comparison, Error> {
            if let Some(failure) = self.fail_with {
                return Err(failure());
            }

            Ok(Comparison::default())
        }
    }

    fn handler(service: SpyService) -> SchemaHandler {
        SchemaHandler::new(Arc::new(service))
    }

    fn caller() -> AuthUser {
        AuthUser(Identity {
            id: Uuid::from_u128(1),
            email: "ada@example.com".to_owned(),
            user_name: "ada".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            avatar_url: None,
            password: String::new(),
            roles: Vec::new(),
        })
    }

    /// What the guard leaves on a request it let through.
    fn signed<T>(message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.extensions_mut().insert(caller());
        request
    }

    #[tokio::test]
    async fn a_create_returns_the_stored_schema() {
        let handler = handler(SpyService::default());

        let response = handler
            .create_schema(signed(v1::CreateSchemaRequest {
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
            .create_schema(signed(v1::CreateSchemaRequest {
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
            .list_schemas(signed(v1::ListSchemasRequest {
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
            .list_schemas(signed(v1::ListSchemasRequest {
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
            .get_schema(signed(v1::GetSchemaRequest {
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
            .validate_schema(signed(v1::ValidateSchemaRequest { target: None }))
            .await
            .expect_err("the call should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn an_unimplemented_rpc_says_so_rather_than_returning_an_empty_result() {
        let handler = handler(SpyService::failing(|| {
            Error::unimplemented("no generator targets mysql yet")
        }));

        let status = handler
            .generate_ddl(signed(v1::GenerateDdlRequest {
                target: Some(v1::generate_ddl_request::Target::Id("s1".to_owned())),
                dialect: v1::Dialect::Postgres as i32,
                include_comments: true,
                include_positions: None,
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
            .delete_schema(signed(v1::DeleteSchemaRequest {
                id: "s1".to_owned(),
            }))
            .await
            .expect("the call should succeed");
    }

    #[tokio::test]
    async fn every_rpc_hands_the_caller_to_the_service() {
        let spy = Arc::new(SpyService::default());
        let handler = SchemaHandler::new(spy.clone());
        let stored = "s1".to_owned();

        handler
            .get_schema(signed(v1::GetSchemaRequest { id: stored.clone() }))
            .await
            .expect("get should succeed");
        handler
            .list_schemas(signed(v1::ListSchemasRequest {
                page: 1,
                per_page: 10,
            }))
            .await
            .expect("list should succeed");
        handler
            .create_schema(signed(v1::CreateSchemaRequest {
                name: "blog".to_owned(),
                description: String::new(),
                entities: vec![],
                relationships: vec![],
            }))
            .await
            .expect("create should succeed");
        handler
            .update_schema(signed(v1::UpdateSchemaRequest {
                id: stored.clone(),
                name: "blog".to_owned(),
                description: String::new(),
                entities: vec![],
                relationships: vec![],
            }))
            .await
            .expect("update should succeed");
        handler
            .delete_schema(signed(v1::DeleteSchemaRequest { id: stored.clone() }))
            .await
            .expect("delete should succeed");
        handler
            .validate_schema(signed(v1::ValidateSchemaRequest {
                target: Some(v1::validate_schema_request::Target::Id(stored.clone())),
            }))
            .await
            .expect("validate should succeed");
        handler
            .generate_ddl(signed(v1::GenerateDdlRequest {
                target: Some(v1::generate_ddl_request::Target::Id(stored.clone())),
                dialect: v1::Dialect::Postgres as i32,
                include_comments: false,
                include_positions: None,
            }))
            .await
            .expect("generate should succeed");
        handler
            .export_schema(signed(v1::ExportSchemaRequest {
                target: Some(v1::export_schema_request::Target::Id(stored)),
                format: v1::ExportFormat::Json as i32,
                include_positions: None,
            }))
            .await
            .expect("export should succeed");

        let callers = spy.callers.lock().expect("the lock should be held");

        assert_eq!(callers.len(), 8);
        assert!(callers.iter().all(|seen| *seen == caller().id()));
    }

    #[tokio::test]
    async fn a_request_without_a_caller_is_turned_away_before_the_service() {
        let handler = handler(SpyService::failing(|| {
            Error::unknown("the service should never be reached")
        }));

        let status = handler
            .get_schema(Request::new(v1::GetSchemaRequest {
                id: "s1".to_owned(),
            }))
            .await
            .expect_err("the call should fail");

        assert_eq!(status.code(), Code::Unauthenticated);
    }
}
