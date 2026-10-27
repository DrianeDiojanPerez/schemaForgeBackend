//! Does a schema survive leaving and coming back? Every fixture goes out as
//! SQL and as JSON and is compared with what it was, one test per fixture
//! per format so a failure says which schema and which format lost what.

use crate::module::schema::core::domain::{ExchangeFormat, Schema, SchemaDraft};
use crate::module::schema::core::ports::{GenerateOptions, Generator};
use crate::module::schema::core::service::fixtures::{self, draft_of};
use crate::module::schema::core::service::PostgresGenerator;

use super::{compare::compare, export, import};

/// Generates the script the canvas would hand to a database, reads it back,
/// and insists the two describe the same diagram.
///
/// A relationship's own name and description are dropped from the
/// expectation first: SQL has nowhere to write them, since the constraint it
/// becomes carries a name of its own and no prose. Everything else is
/// expected to survive.
fn through_sql(schema: &Schema) {
    let script = PostgresGenerator.generate(schema, GenerateOptions::default());
    let (actual, report) = import(&script, ExchangeFormat::Sql, true).expect("the script loads");

    assert!(
        report.diagnostics.is_empty(),
        "the generator's own output should read back whole: {:?}",
        report.diagnostics
    );

    let mut expected = draft_of(schema);

    for relationship in &mut expected.relationships {
        relationship.name = String::new();
        relationship.description = String::new();
    }

    equal(&expected, &actual, "sql");
}

fn through_json(schema: &Schema) {
    let file = export(schema, ExchangeFormat::Json, true).expect("the export succeeds");
    let (actual, _) = import(&file.content, ExchangeFormat::Json, true).expect("the file loads");

    equal(&draft_of(schema), &actual, "json");
}

fn equal(expected: &SchemaDraft, actual: &SchemaDraft, format: &str) {
    let comparison = compare(expected, actual, true);

    assert!(
        comparison.is_equal(),
        "{} did not survive {format}: {:#?}",
        expected.name,
        comparison.differences
    );
}

#[test]
fn course_registration_survives_the_generated_script() {
    through_sql(&fixtures::course_registration());
}

#[test]
fn course_registration_survives_the_file() {
    through_json(&fixtures::course_registration());
}

#[test]
fn reserved_words_survive_the_generated_script() {
    through_sql(&fixtures::reserved_words());
}

#[test]
fn reserved_words_survive_the_file() {
    through_json(&fixtures::reserved_words());
}

#[test]
fn a_composite_key_survives_the_generated_script() {
    through_sql(&fixtures::composite_key());
}

#[test]
fn a_composite_key_survives_the_file() {
    through_json(&fixtures::composite_key());
}

#[test]
fn a_self_reference_survives_the_generated_script() {
    through_sql(&fixtures::self_reference());
}

#[test]
fn a_self_reference_survives_the_file() {
    through_json(&fixtures::self_reference());
}

#[test]
fn a_cycle_of_nullable_keys_survives_the_generated_script() {
    through_sql(&fixtures::nullable_cycle());
}

#[test]
fn a_cycle_of_nullable_keys_survives_the_file() {
    through_json(&fixtures::nullable_cycle());
}

#[test]
fn names_too_long_for_a_constraint_survive_the_generated_script() {
    through_sql(&fixtures::long_names());
}

#[test]
fn names_too_long_for_a_constraint_survive_the_file() {
    through_json(&fixtures::long_names());
}

#[test]
fn prose_full_of_quotes_survives_the_generated_script() {
    through_sql(&fixtures::quoted_prose());
}

#[test]
fn prose_full_of_quotes_survives_the_file() {
    through_json(&fixtures::quoted_prose());
}

/// The relationship prose SQL cannot carry is the only thing excluded above,
/// so it is worth seeing it lost rather than taking the exclusion on trust.
#[test]
fn the_prose_on_a_relationship_is_what_sql_leaves_behind() {
    let schema = fixtures::course_registration();
    let script = PostgresGenerator.generate(&schema, GenerateOptions::default());
    let (actual, _) = import(&script, ExchangeFormat::Sql, true).expect("the script loads");
    let comparison = compare(&draft_of(&schema), &actual, true);

    assert_eq!(comparison.descriptions.missing, 1);
    assert_eq!(
        comparison.differences[0].expected,
        "A student may enrol many times"
    );
}
