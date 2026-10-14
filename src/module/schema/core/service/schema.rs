use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::module::schema::core::domain::{
    Dialect, DomainError, Report, Schema, SchemaDraft, SchemaSummary,
};
use crate::module::schema::core::ports::{
    ExportRequest, Exported, GenerateRequest, Generated, Generator, ImportRequest, Imported,
    SchemaRepository, SchemaService, ValidationTarget, Verifier,
};
use crate::package::errdef::Error;
use crate::package::pagination::{Data, ListRequest};

use super::exchange;

const MAX_NAME_LENGTH: usize = 120;

pub struct SchemaServiceImpl {
    repository: Arc<dyn SchemaRepository>,
    verifier: Arc<dyn Verifier>,
    generators: Vec<Arc<dyn Generator>>,
}

impl SchemaServiceImpl {
    pub fn new(
        repository: Arc<dyn SchemaRepository>,
        verifier: Arc<dyn Verifier>,
        generators: Vec<Arc<dyn Generator>>,
    ) -> Self {
        Self {
            repository,
            verifier,
            generators,
        }
    }

    async fn resolve(&self, target: ValidationTarget) -> Result<Schema, Error> {
        match target {
            ValidationTarget::Stored(id) => self.find_by_id(&id).await,
            ValidationTarget::Draft(schema) => Ok(*schema),
        }
    }

    /// A dialect nobody has written a generator for is a gap in the tool
    /// rather than a mistake by the caller, which is why it answers
    /// unimplemented instead of a validation failure.
    fn generator_for(&self, dialect: Dialect) -> Result<&dyn Generator, Error> {
        self.generators
            .iter()
            .find(|generator| generator.dialect() == dialect)
            .map(Arc::as_ref)
            .ok_or_else(|| Error::unimplemented(format!("no generator targets {dialect} yet")))
    }

    /// Payload-level checks only: whether the request itself is usable. The
    /// schema's own correctness is the verifier's job, and it runs against the
    /// canonical model rather than against a request.
    fn check_draft(draft: &SchemaDraft) -> Result<(), Error> {
        let mut error = Error::validation("failed payload validation");

        if draft.name.trim().is_empty() {
            error.push_violation("name", "field is required and cannot be empty");
        }

        if draft.name.chars().count() > MAX_NAME_LENGTH {
            error.push_violation(
                "name",
                format!("field must be at most {MAX_NAME_LENGTH} characters"),
            );
        }

        if error.has_violations() {
            return Err(error);
        }

        Ok(())
    }
}

/// The store speaks its own failures; the transport speaks status codes. This
/// is the one place the two are joined, so no adapter has to know about the
/// other's vocabulary.
fn to_error(failure: DomainError) -> Error {
    match failure {
        DomainError::SchemaNotFound => Error::not_found("schema does not exist"),
        DomainError::DuplicateSchemaName(name) => {
            Error::conflict(format!("a schema named `{name}` already exists"))
        }
        other => Error::unknown(other),
    }
}

#[async_trait]
impl SchemaService for SchemaServiceImpl {
    #[tracing::instrument(name = "SchemaService.Index", skip_all)]
    async fn index(&self, request: ListRequest) -> Result<Data<SchemaSummary>, Error> {
        let (summaries, total) = self.repository.index(&request).await.map_err(to_error)?;

        Ok(Data::new(summaries, total, request.page, request.per_page))
    }

    #[tracing::instrument(name = "SchemaService.Create", skip_all, fields(schema.name = %draft.name))]
    async fn create(&self, draft: SchemaDraft) -> Result<Schema, Error> {
        Self::check_draft(&draft)?;

        self.repository.create(draft).await.map_err(to_error)
    }

    #[tracing::instrument(name = "SchemaService.FindById", skip_all, fields(schema.id = %id))]
    async fn find_by_id(&self, id: &str) -> Result<Schema, Error> {
        self.repository
            .find_by_id(id)
            .await
            .map_err(to_error)?
            .ok_or_else(|| Error::not_found("schema does not exist"))
    }

    #[tracing::instrument(name = "SchemaService.Replace", skip_all, fields(schema.id = %id))]
    async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, Error> {
        Self::check_draft(&draft)?;

        self.repository.replace(id, draft).await.map_err(to_error)
    }

    #[tracing::instrument(name = "SchemaService.Delete", skip_all, fields(schema.id = %id))]
    async fn delete(&self, id: &str) -> Result<(), Error> {
        self.repository.delete(id).await.map_err(to_error)
    }

    #[tracing::instrument(name = "SchemaService.Validate", skip_all)]
    async fn validate(&self, target: ValidationTarget) -> Result<Report, Error> {
        let schema = self.resolve(target).await?;

        Ok(self.verifier.verify(&schema))
    }

    #[tracing::instrument(name = "SchemaService.GenerateDdl", skip_all)]
    async fn generate_ddl(&self, request: GenerateRequest) -> Result<Generated, Error> {
        let generator = self.generator_for(request.dialect)?;

        let schema = self.resolve(request.target).await?;
        let report = self.verifier.verify(&schema);

        // SQL built from a broken diagram is SQL no database would accept, so
        // a caller who skipped validation gets the same diagnostics back in
        // place of the script rather than something that looks usable.
        if !report.is_valid() {
            return Ok(Generated {
                ddl: String::new(),
                report,
            });
        }

        Ok(Generated {
            ddl: generator.generate(&schema, request.options),
            report,
        })
    }

    #[tracing::instrument(name = "SchemaService.Export", skip_all)]
    async fn export(&self, request: ExportRequest) -> Result<Exported, Error> {
        let schema = self.resolve(request.target).await?;

        // Exporting is not generating. A half-drawn diagram is worth saving
        // to a file and coming back to, so nothing is verified here.
        exchange::export(&schema, request.format, request.include_positions)
    }

    #[tracing::instrument(name = "SchemaService.Import", skip_all)]
    async fn import(&self, request: ImportRequest) -> Result<Imported, Error> {
        let (draft, report) =
            exchange::import(&request.content, request.format, request.include_positions)?;

        // The draft has no id because nothing was stored. The canvas decides
        // whether this becomes a schema, the same as any other diagram it is
        // holding.
        Ok(Imported {
            draft: Schema::new(String::new(), draft, Utc::now()),
            report,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use chrono::{DateTime, Utc};

    use crate::module::schema::core::domain::{
        diagnostic_code, Attribute, DataType, DataTypeKind, Diagnostic, Entity, ExchangeFormat,
    };
    use crate::module::schema::core::ports::GenerateOptions;
    use crate::module::schema::core::service::{PostgresGenerator, SchemaVerifier};
    use crate::package::errdef::code;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant")
    }

    #[derive(Default)]
    struct FakeRepository {
        schemas: Mutex<Vec<Schema>>,
        fail_with: Option<fn() -> DomainError>,
    }

    impl FakeRepository {
        fn holding(schemas: Vec<Schema>) -> Self {
            Self {
                schemas: Mutex::new(schemas),
                fail_with: None,
            }
        }

        fn failing(failure: fn() -> DomainError) -> Self {
            Self {
                schemas: Mutex::new(Vec::new()),
                fail_with: Some(failure),
            }
        }

        fn guard(&self) -> Result<(), DomainError> {
            match self.fail_with {
                Some(failure) => Err(failure()),
                None => Ok(()),
            }
        }
    }

    #[async_trait]
    impl SchemaRepository for FakeRepository {
        async fn index(
            &self,
            request: &ListRequest,
        ) -> Result<(Vec<SchemaSummary>, i64), DomainError> {
            self.guard()?;

            let schemas = self.schemas.lock().expect("the lock should be held");
            let total = schemas.len() as i64;
            let page = schemas
                .iter()
                .skip(request.offset() as usize)
                .take(request.per_page as usize)
                .map(Schema::summary)
                .collect();

            Ok((page, total))
        }

        async fn create(&self, draft: SchemaDraft) -> Result<Schema, DomainError> {
            self.guard()?;

            let schema = Schema::new("s-new", draft, now());
            self.schemas
                .lock()
                .expect("the lock should be held")
                .push(schema.clone());

            Ok(schema)
        }

        async fn find_by_id(&self, id: &str) -> Result<Option<Schema>, DomainError> {
            self.guard()?;

            Ok(self
                .schemas
                .lock()
                .expect("the lock should be held")
                .iter()
                .find(|schema| schema.id == id)
                .cloned())
        }

        async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, DomainError> {
            self.guard()?;

            Ok(self
                .schemas
                .lock()
                .expect("the lock should be held")
                .iter()
                .find(|schema| schema.name == name)
                .cloned())
        }

        async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, DomainError> {
            self.guard()?;

            let mut schemas = self.schemas.lock().expect("the lock should be held");
            let schema = schemas
                .iter_mut()
                .find(|schema| schema.id == id)
                .ok_or(DomainError::SchemaNotFound)?;

            schema.replace_with(draft, now());

            Ok(schema.clone())
        }

        async fn delete(&self, id: &str) -> Result<(), DomainError> {
            self.guard()?;

            let mut schemas = self.schemas.lock().expect("the lock should be held");
            let before = schemas.len();
            schemas.retain(|schema| schema.id != id);

            if schemas.len() == before {
                return Err(DomainError::SchemaNotFound);
            }

            Ok(())
        }
    }

    fn service(repository: FakeRepository) -> SchemaServiceImpl {
        SchemaServiceImpl::new(
            Arc::new(repository),
            Arc::new(SchemaVerifier),
            vec![Arc::new(PostgresGenerator)],
        )
    }

    struct SeenVerifier(Mutex<Option<String>>);

    impl Verifier for SeenVerifier {
        fn verify(&self, schema: &Schema) -> Report {
            *self.0.lock().expect("the lock should be held") = Some(schema.id.clone());

            Report::new(vec![Diagnostic::error(
                diagnostic_code::MISSING_PRIMARY_KEY,
                "users has no primary key",
            )
            .about("e1")])
        }
    }

    fn draft(name: &str) -> SchemaDraft {
        SchemaDraft {
            name: name.to_owned(),
            description: String::new(),
            entities: vec![Entity::new("e1", "users")],
            relationships: Vec::new(),
        }
    }

    fn stored(id: &str, name: &str) -> Schema {
        Schema::new(id, draft(name), now())
    }

    /// The smallest schema the verifier passes, so a generation test is about
    /// the SQL rather than about getting a diagram past the checks.
    fn blog(id: &str) -> Schema {
        Schema::new(
            id,
            SchemaDraft {
                name: "blog".to_owned(),
                description: String::new(),
                entities: vec![Entity::new("e1", "users")
                    .with_description("Registered accounts")
                    .with_attributes(vec![Attribute::new(
                        "a1",
                        "id",
                        DataType::simple(DataTypeKind::Uuid),
                    )
                    .as_primary_key()
                    .with_description("Surrogate key")])],
                relationships: Vec::new(),
            },
            now(),
        )
    }

    #[tokio::test]
    async fn creates_a_schema_from_a_valid_draft() {
        let service = service(FakeRepository::default());

        let schema = service
            .create(draft("blog"))
            .await
            .expect("creation should succeed");

        assert_eq!(schema.name, "blog");
        assert_eq!(schema.entities.len(), 1);
    }

    #[tokio::test]
    async fn a_nameless_schema_is_rejected_before_the_store_is_touched() {
        let service = service(FakeRepository::failing(|| {
            DomainError::Storage("the store should never be reached".to_owned())
        }));

        let error = service
            .create(draft("   "))
            .await
            .expect_err("creation should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
        assert!(error.has_violations());
    }

    #[tokio::test]
    async fn an_over_long_name_is_rejected() {
        let service = service(FakeRepository::default());

        let error = service
            .create(draft(&"n".repeat(MAX_NAME_LENGTH + 1)))
            .await
            .expect_err("creation should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn a_name_at_the_limit_is_accepted() {
        let service = service(FakeRepository::default());

        assert!(service
            .create(draft(&"n".repeat(MAX_NAME_LENGTH)))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_missing_schema_is_a_not_found_not_an_empty_result() {
        let service = service(FakeRepository::default());

        let error = service
            .find_by_id("nope")
            .await
            .expect_err("the lookup should fail");

        assert_eq!(error.app_code(), code::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_duplicate_name_surfaces_as_a_conflict() {
        let service = service(FakeRepository::failing(|| {
            DomainError::DuplicateSchemaName("blog".to_owned())
        }));

        let error = service
            .create(draft("blog"))
            .await
            .expect_err("creation should fail");

        assert_eq!(error.app_code(), code::RESOURCE_CONFLICT);
        assert!(error.to_string().contains("blog"));
    }

    #[tokio::test]
    async fn a_storage_failure_stays_internal() {
        let service = service(FakeRepository::failing(|| {
            DomainError::Storage("disk on fire at 10.0.0.1".to_owned())
        }));

        let error = service
            .find_by_id("s1")
            .await
            .expect_err("the lookup should fail");

        assert_eq!(error.app_code(), code::UNKNOWN);
        assert!(
            !error.to_string().contains("10.0.0.1") || error.to_string().contains("cause"),
            "the address may appear on the log line but never in the message"
        );
    }

    #[tokio::test]
    async fn replacing_keeps_the_identity_it_was_given() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        let schema = service
            .replace("s1", draft("renamed"))
            .await
            .expect("the replace should succeed");

        assert_eq!(schema.id, "s1");
        assert_eq!(schema.name, "renamed");
    }

    #[tokio::test]
    async fn replacing_validates_the_draft_too() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        let error = service
            .replace("s1", draft(""))
            .await
            .expect_err("the replace should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn deleting_a_missing_schema_is_a_not_found() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        assert!(service.delete("s1").await.is_ok());
        assert_eq!(
            service
                .delete("s1")
                .await
                .expect_err("the second delete should fail")
                .app_code(),
            code::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_listing_reports_the_total_alongside_the_page() {
        let service = service(FakeRepository::holding(vec![
            stored("s1", "one"),
            stored("s2", "two"),
            stored("s3", "three"),
        ]));

        let page = service
            .index(ListRequest::from_query("page=1&per_page=2"))
            .await
            .expect("the listing should succeed");

        assert_eq!(page.data.len(), 2, "the page is capped at the page size");
        assert_eq!(page.meta.total_count, 3, "the total counts every schema");
    }

    #[tokio::test]
    async fn validating_a_missing_schema_fails_before_the_engine_is_reached() {
        let service = service(FakeRepository::default());

        let error = service
            .validate(ValidationTarget::Stored("nope".to_owned()))
            .await
            .expect_err("validation should fail");

        assert_eq!(
            error.app_code(),
            code::NOT_FOUND,
            "resolving the target comes first, so a bad id is a not-found rather than unimplemented"
        );
    }

    #[tokio::test]
    async fn validating_a_stored_schema_hands_it_to_the_verifier() {
        let verifier = Arc::new(SeenVerifier(Mutex::new(None)));
        let service = SchemaServiceImpl::new(
            Arc::new(FakeRepository::holding(vec![stored("s1", "blog")])),
            verifier.clone(),
            vec![Arc::new(PostgresGenerator)],
        );

        let report = service
            .validate(ValidationTarget::Stored("s1".to_owned()))
            .await
            .expect("validation should run");

        assert_eq!(
            verifier
                .0
                .lock()
                .expect("the lock should be held")
                .as_deref(),
            Some("s1")
        );
        assert!(!report.is_valid());
        assert!(report.has(diagnostic_code::MISSING_PRIMARY_KEY));
    }

    #[tokio::test]
    async fn a_draft_is_verified_without_being_stored() {
        let repository = Arc::new(FakeRepository::default());
        let service = SchemaServiceImpl::new(
            repository.clone(),
            Arc::new(SchemaVerifier),
            vec![Arc::new(PostgresGenerator)],
        );

        let report = service
            .validate(ValidationTarget::Draft(Box::new(stored("", "draft"))))
            .await
            .expect("validation should run");

        assert!(
            report.has(diagnostic_code::MISSING_PRIMARY_KEY),
            "the draft's users table has no key"
        );
        assert!(repository
            .schemas
            .lock()
            .expect("the lock should be held")
            .is_empty());
    }

    fn generate(service: &SchemaServiceImpl, id: &str) -> GenerateRequest {
        GenerateRequest {
            target: ValidationTarget::Stored(id.to_owned()),
            dialect: service.generators[0].dialect(),
            options: GenerateOptions::default(),
        }
    }

    #[tokio::test]
    async fn a_valid_schema_generates_sql_for_the_dialect_that_was_asked_for() {
        let service = service(FakeRepository::holding(vec![blog("s1")]));

        let generated = service
            .generate_ddl(generate(&service, "s1"))
            .await
            .expect("generation should run");

        assert!(generated.report.is_valid());
        assert!(generated.ddl.contains("CREATE TABLE \"users\""));
        assert!(generated.ddl.contains("COMMENT ON TABLE \"users\""));
    }

    #[tokio::test]
    async fn a_broken_schema_comes_back_as_diagnostics_rather_than_sql() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        let generated = service
            .generate_ddl(generate(&service, "s1"))
            .await
            .expect("a broken schema is an answer, not a failure");

        assert!(generated.ddl.is_empty(), "got:\n{}", generated.ddl);
        assert!(generated.report.has(diagnostic_code::MISSING_PRIMARY_KEY));
    }

    #[tokio::test]
    async fn an_undocumented_schema_still_generates() {
        let mut schema = blog("s1");
        schema.entities[0].description = String::new();

        let service = service(FakeRepository::holding(vec![schema]));

        let generated = service
            .generate_ddl(generate(&service, "s1"))
            .await
            .expect("generation should run");

        assert!(
            generated.report.has(diagnostic_code::MISSING_DESCRIPTION),
            "the warning is still reported"
        );
        assert!(
            !generated.ddl.is_empty(),
            "a warning does not block generation"
        );
    }

    #[tokio::test]
    async fn a_dialect_nobody_has_written_a_generator_for_says_so() {
        let service = service(FakeRepository::holding(vec![blog("s1")]));

        let error = service
            .generate_ddl(GenerateRequest {
                target: ValidationTarget::Stored("s1".to_owned()),
                dialect: Dialect::MySql,
                options: GenerateOptions::default(),
            })
            .await
            .expect_err("only postgres is registered");

        assert_eq!(error.app_code(), code::UNIMPLEMENTED);
        assert!(error.to_string().contains("mysql"));
    }

    #[tokio::test]
    async fn a_broken_schema_still_exports_because_a_file_is_not_a_database() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        let exported = service
            .export(ExportRequest {
                target: ValidationTarget::Stored("s1".to_owned()),
                format: ExchangeFormat::Json,
                include_positions: true,
            })
            .await
            .expect("export should run on a half-drawn diagram");

        assert_eq!(exported.file_name, "blog.json");
        assert!(exported.content.contains("\"schemaforge\": 1"));
    }

    #[tokio::test]
    async fn a_draft_is_exported_without_being_stored() {
        let repository = Arc::new(FakeRepository::default());
        let service = SchemaServiceImpl::new(
            repository.clone(),
            Arc::new(SchemaVerifier),
            vec![Arc::new(PostgresGenerator)],
        );

        let exported = service
            .export(ExportRequest {
                target: ValidationTarget::Draft(Box::new(blog("s1"))),
                format: ExchangeFormat::Json,
                include_positions: true,
            })
            .await
            .expect("export should run");

        assert!(exported.content.contains("\"users\""));
        assert!(repository
            .schemas
            .lock()
            .expect("the lock should be held")
            .is_empty());
    }

    #[tokio::test]
    async fn exporting_a_missing_schema_is_a_not_found() {
        let service = service(FakeRepository::default());

        let error = service
            .export(ExportRequest {
                target: ValidationTarget::Stored("nope".to_owned()),
                format: ExchangeFormat::Json,
                include_positions: true,
            })
            .await
            .expect_err("export should fail");

        assert_eq!(error.app_code(), code::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_imported_file_comes_back_as_a_draft_with_no_id() {
        let repository = Arc::new(FakeRepository::default());
        let service = SchemaServiceImpl::new(
            repository.clone(),
            Arc::new(SchemaVerifier),
            vec![Arc::new(PostgresGenerator)],
        );

        let imported = service
            .import(ImportRequest {
                content: r#"{ "name": "blog", "entities": [] }"#.to_owned(),
                format: ExchangeFormat::Json,
                include_positions: true,
            })
            .await
            .expect("the file should load");

        assert_eq!(imported.draft.name, "blog");
        assert!(imported.draft.id.is_empty(), "nothing was stored");
        assert!(repository
            .schemas
            .lock()
            .expect("the lock should be held")
            .is_empty());
    }

    #[tokio::test]
    async fn a_file_the_reader_cannot_make_sense_of_is_refused() {
        let service = service(FakeRepository::default());

        let error = service
            .import(ImportRequest {
                content: "not a schema".to_owned(),
                format: ExchangeFormat::Json,
                include_positions: true,
            })
            .await
            .expect_err("the file should be refused");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn generating_from_a_missing_schema_is_a_not_found() {
        let service = service(FakeRepository::default());

        let error = service
            .generate_ddl(generate(&service, "nope"))
            .await
            .expect_err("generation should fail");

        assert_eq!(error.app_code(), code::NOT_FOUND);
    }
}
