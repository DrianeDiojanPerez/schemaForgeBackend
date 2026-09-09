use std::sync::Arc;

use async_trait::async_trait;

use crate::module::schema::core::domain::{
    DomainError, Report, Schema, SchemaDraft, SchemaSummary,
};
use crate::module::schema::core::ports::{
    GenerateRequest, Generated, SchemaRepository, SchemaService, ValidationTarget,
};
use crate::package::errdef::Error;
use crate::package::pagination::{Data, ListRequest};

const MAX_NAME_LENGTH: usize = 120;

pub struct SchemaServiceImpl {
    repository: Arc<dyn SchemaRepository>,
}

impl SchemaServiceImpl {
    pub fn new(repository: Arc<dyn SchemaRepository>) -> Self {
        Self { repository }
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
        // Resolving the target now rather than at the call site means the
        // engine landing in M2 only has to be plugged in below: a stored id is
        // already proven to exist, and a draft is already in canonical form.
        let _schema = match target {
            ValidationTarget::Stored(id) => self.find_by_id(&id).await?,
            ValidationTarget::Draft(schema) => *schema,
        };

        Err(Error::unimplemented(
            "ValidateSchema lands in milestone M2 (weeks 5-8)",
        ))
    }

    #[tracing::instrument(name = "SchemaService.GenerateDdl", skip_all)]
    async fn generate_ddl(&self, request: GenerateRequest) -> Result<Generated, Error> {
        let _schema = match request.target {
            ValidationTarget::Stored(id) => self.find_by_id(&id).await?,
            ValidationTarget::Draft(schema) => *schema,
        };

        Err(Error::unimplemented(
            "GenerateDdl lands in milestone M3 (weeks 7-11)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use chrono::{DateTime, Utc};

    use crate::module::schema::core::domain::{Dialect, Entity};
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
        SchemaServiceImpl::new(Arc::new(repository))
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
    async fn validating_a_resolvable_target_reports_the_milestone_it_waits_on() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        let error = service
            .validate(ValidationTarget::Stored("s1".to_owned()))
            .await
            .expect_err("the engine is not built yet");

        assert_eq!(error.app_code(), code::UNIMPLEMENTED);
        assert!(error.to_string().contains("M2"));
    }

    #[tokio::test]
    async fn generation_reports_the_milestone_it_waits_on() {
        let service = service(FakeRepository::holding(vec![stored("s1", "blog")]));

        let error = service
            .generate_ddl(GenerateRequest {
                target: ValidationTarget::Stored("s1".to_owned()),
                dialect: Dialect::Postgres,
                include_comments: true,
            })
            .await
            .expect_err("the generator is not built yet");

        assert_eq!(error.app_code(), code::UNIMPLEMENTED);
        assert!(error.to_string().contains("M3"));
    }
}
