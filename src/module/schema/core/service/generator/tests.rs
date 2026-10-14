use chrono::DateTime;

use crate::module::schema::core::domain::{
    Attribute, DataType, DataTypeKind, Entity, ForeignKeyRef, Position, Schema, SchemaDraft,
};
use crate::module::schema::core::ports::{GenerateOptions, Generator};

use super::PostgresGenerator;

fn key(name: &str) -> Attribute {
    Attribute::new(
        format!("a.{name}"),
        name,
        DataType::simple(DataTypeKind::Uuid),
    )
    .as_primary_key()
}

fn column(name: &str, data_type: DataType) -> Attribute {
    Attribute::new(format!("a.{name}"), name, data_type)
}

fn reference(name: &str, entity: &str, attribute: &str) -> Attribute {
    column(name, DataType::simple(DataTypeKind::Uuid)).referencing(ForeignKeyRef::new(
        format!("e.{entity}"),
        format!("a.{attribute}"),
    ))
}

fn table(name: &str, attributes: Vec<Attribute>) -> Entity {
    Entity::new(format!("e.{name}"), name).with_attributes(attributes)
}

fn schema(entities: Vec<Entity>) -> Schema {
    Schema::new(
        "s1",
        SchemaDraft {
            name: "case study".to_owned(),
            description: String::new(),
            entities,
            relationships: Vec::new(),
        },
        DateTime::from_timestamp(0, 0).expect("a fixed instant"),
    )
}

fn options(include_comments: bool, include_positions: bool) -> GenerateOptions {
    GenerateOptions {
        include_comments,
        include_positions,
    }
}

fn ddl(entities: Vec<Entity>) -> String {
    PostgresGenerator.generate(&schema(entities), options(false, false))
}

fn documented_ddl(entities: Vec<Entity>) -> String {
    PostgresGenerator.generate(&schema(entities), options(true, false))
}

fn positioned_ddl(entities: Vec<Entity>) -> String {
    PostgresGenerator.generate(&schema(entities), options(false, true))
}

#[test]
fn the_generator_names_the_dialect_it_targets() {
    use crate::module::schema::core::domain::Dialect;

    assert_eq!(PostgresGenerator.dialect(), Dialect::Postgres);
}

#[test]
fn a_table_is_emitted_with_its_columns_and_its_key() {
    let sql = ddl(vec![table(
        "students",
        vec![
            key("id"),
            column("full_name", DataType::simple(DataTypeKind::Text)).required(),
            column("email", DataType::varchar(255)),
        ],
    )]);

    assert_eq!(
        sql,
        concat!(
            "CREATE TABLE \"students\" (\n",
            "    \"id\" UUID NOT NULL,\n",
            "    \"full_name\" TEXT NOT NULL,\n",
            "    \"email\" VARCHAR(255),\n",
            "    PRIMARY KEY (\"id\")\n",
            ");"
        )
    );
}

#[test]
fn a_composite_key_is_one_constraint_at_the_foot_of_the_table() {
    let sql = ddl(vec![table(
        "enrollments",
        vec![key("student_id"), key("course_id")],
    )]);

    assert!(
        sql.contains("PRIMARY KEY (\"student_id\", \"course_id\")"),
        "both halves belong to one constraint, got:\n{sql}"
    );
    assert_eq!(sql.matches("PRIMARY KEY").count(), 1);
}

#[test]
fn a_reserved_word_is_still_a_usable_table_name() {
    let sql = ddl(vec![table("order", vec![key("id")])]);

    assert!(
        sql.starts_with("CREATE TABLE \"order\" ("),
        "quoting is what makes a reserved word safe, got:\n{sql}"
    );
}

#[test]
fn a_name_the_user_drew_in_mixed_case_keeps_its_case() {
    let sql = ddl(vec![table("Students", vec![key("studentId")])]);

    assert!(sql.contains("\"Students\""));
    assert!(sql.contains("\"studentId\""));
}

#[test]
fn a_quote_inside_a_name_is_escaped_rather_than_closing_the_identifier() {
    let sql = ddl(vec![table("od\"d", vec![key("id")])]);

    assert!(
        sql.contains("\"od\"\"d\""),
        "a doubled quote keeps the identifier intact, got:\n{sql}"
    );
}

#[test]
fn a_default_goes_through_as_written() {
    let sql = ddl(vec![table(
        "accounts",
        vec![
            key("id"),
            column("status", DataType::varchar(20)).with_default("'active'"),
            column("created_at", DataType::simple(DataTypeKind::TimestampTz)).with_default("now()"),
        ],
    )]);

    assert!(sql.contains("\"status\" VARCHAR(20) DEFAULT 'active'"));
    assert!(
        sql.contains("\"created_at\" TIMESTAMPTZ DEFAULT now()"),
        "a call is a default like any other, got:\n{sql}"
    );
}

#[test]
fn a_unique_column_that_is_not_the_key_says_so() {
    let sql = ddl(vec![table(
        "students",
        vec![key("id"), column("email", DataType::varchar(255)).unique()],
    )]);

    assert!(sql.contains("\"email\" VARCHAR(255) UNIQUE"));
}

#[test]
fn the_key_column_does_not_repeat_itself_as_unique() {
    let sql = ddl(vec![table("students", vec![key("id").unique()])]);

    assert!(
        !sql.contains("\"id\" UUID NOT NULL UNIQUE"),
        "the primary key constraint already says it, got:\n{sql}"
    );
}

#[test]
fn a_numeric_keeps_both_of_its_parameters() {
    let sql = ddl(vec![table(
        "invoices",
        vec![key("id"), column("total", DataType::numeric(10, 2))],
    )]);

    assert!(sql.contains("\"total\" NUMERIC(10, 2)"));
}

#[test]
fn a_table_with_nothing_drawn_on_it_is_still_a_statement() {
    let sql = ddl(vec![table("blank", Vec::new())]);

    assert_eq!(sql, "CREATE TABLE \"blank\" ();");
}

#[test]
fn tables_come_out_in_the_order_they_were_drawn() {
    let sql = ddl(vec![
        table("students", vec![key("id")]),
        table("courses", vec![key("id")]),
    ]);

    let students = sql.find("\"students\"").expect("students is emitted");
    let courses = sql.find("\"courses\"").expect("courses is emitted");

    assert!(students < courses);
    assert!(sql.contains(");\n\nCREATE TABLE"), "got:\n{sql}");
}

#[test]
fn an_empty_schema_generates_nothing_rather_than_whitespace() {
    assert_eq!(ddl(Vec::new()), "");
}

#[test]
fn a_foreign_key_is_added_after_every_table_is_created() {
    let sql = ddl(vec![
        table(
            "posts",
            vec![key("id"), reference("user_id", "users", "id")],
        ),
        table("users", vec![key("id")]),
    ]);

    let create_users = sql
        .find("CREATE TABLE \"users\"")
        .expect("users is created");
    let alter = sql.find("ALTER TABLE").expect("the key is added");

    assert!(
        create_users < alter,
        "posts was drawn first but still references a table that exists by then, got:\n{sql}"
    );
}

#[test]
fn a_foreign_key_names_the_table_and_column_it_points_at() {
    let sql = ddl(vec![
        table("users", vec![key("id")]),
        table(
            "posts",
            vec![key("id"), reference("user_id", "users", "id")],
        ),
    ]);

    assert!(
        sql.contains(concat!(
            "ALTER TABLE \"posts\"\n",
            "    ADD CONSTRAINT \"fk_posts_user_id\" FOREIGN KEY (\"user_id\") ",
            "REFERENCES \"users\" (\"id\");"
        )),
        "got:\n{sql}"
    );
}

#[test]
fn a_column_holding_no_reference_adds_no_constraint() {
    let sql = ddl(vec![table(
        "users",
        vec![key("id"), column("email", DataType::varchar(255))],
    )]);

    assert!(!sql.contains("ALTER TABLE"));
}

#[test]
fn a_reference_to_a_column_that_is_not_there_emits_nothing() {
    let sql = ddl(vec![table(
        "posts",
        vec![key("id"), reference("user_id", "users", "id")],
    )]);

    assert!(
        !sql.contains("ALTER TABLE"),
        "the verifier already refuses this schema, got:\n{sql}"
    );
}

#[test]
fn a_constraint_name_is_cut_to_what_postgres_will_store() {
    let long = "a".repeat(80);
    let sql = ddl(vec![
        table("users", vec![key("id")]),
        table(&long, vec![key("id"), reference("user_id", "users", "id")]),
    ]);

    let name = sql
        .split("ADD CONSTRAINT \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a constraint is emitted");

    assert_eq!(name.len(), 63, "got `{name}`");
    assert!(name.starts_with("fk_aaa"));
}

#[test]
fn every_reference_in_the_schema_gets_its_own_constraint() {
    let sql = ddl(vec![
        table("students", vec![key("id")]),
        table("courses", vec![key("id")]),
        table(
            "enrollments",
            vec![
                key("id"),
                reference("student_id", "students", "id"),
                reference("course_id", "courses", "id"),
            ],
        ),
    ]);

    assert_eq!(sql.matches("ADD CONSTRAINT").count(), 2);
    assert!(sql.contains("\"fk_enrollments_student_id\""));
    assert!(sql.contains("\"fk_enrollments_course_id\""));
}

#[test]
fn a_described_table_and_column_travel_as_comments() {
    let sql = documented_ddl(vec![table(
        "students",
        vec![key("id").with_description("Surrogate key")],
    )
    .with_description("Everyone enrolled this term")]);

    assert!(sql.contains("COMMENT ON TABLE \"students\" IS 'Everyone enrolled this term';"));
    assert!(sql.contains("COMMENT ON COLUMN \"students\".\"id\" IS 'Surrogate key';"));
}

#[test]
fn descriptions_stay_behind_when_they_are_not_asked_for() {
    let sql = ddl(vec![table(
        "students",
        vec![key("id").with_description("Surrogate key")],
    )
    .with_description("Everyone enrolled this term")]);

    assert!(!sql.contains("COMMENT ON"));
}

#[test]
fn an_undocumented_column_gets_no_comment() {
    let sql = documented_ddl(vec![table(
        "students",
        vec![key("id"), column("email", DataType::varchar(255))],
    )]);

    assert!(
        !sql.contains("COMMENT ON"),
        "nothing was described, got:\n{sql}"
    );
}

#[test]
fn a_blank_description_is_not_worth_a_comment() {
    let sql = documented_ddl(vec![table(
        "students",
        vec![key("id").with_description("   ")],
    )
    .with_description("  ")]);

    assert!(!sql.contains("COMMENT ON"));
}

#[test]
fn an_apostrophe_in_a_description_is_doubled_rather_than_closing_the_string() {
    let sql = documented_ddl(vec![
        table("students", vec![key("id")]).with_description("The student's record")
    ]);

    assert!(sql.contains("IS 'The student''s record';"), "got:\n{sql}");
}

#[test]
fn the_comments_come_after_the_tables_they_describe() {
    let sql = documented_ddl(vec![
        table("students", vec![key("id")]).with_description("Everyone enrolled")
    ]);

    let create = sql.find("CREATE TABLE").expect("the table is created");
    let comment = sql.find("COMMENT ON").expect("the comment is emitted");

    assert!(create < comment);
}

#[test]
fn the_canvas_layout_rides_along_as_comments() {
    let sql = positioned_ddl(vec![
        table("users", vec![key("id")]).with_position(Position::new(120.0, 340.0)),
        table("orders", vec![key("id")]).with_position(Position::new(560.0, 340.0)),
    ]);

    assert!(
        sql.ends_with(concat!(
            "-- @schemaforge layout 1\n",
            "-- @table \"users\" 120 340\n",
            "-- @table \"orders\" 560 340"
        )),
        "got:\n{sql}"
    );
}

#[test]
fn a_position_lands_on_whole_numbers() {
    let sql = positioned_ddl(vec![
        table("users", vec![key("id")]).with_position(Position::new(119.6, -340.4))
    ]);

    assert!(sql.contains("-- @table \"users\" 120 -340"), "got:\n{sql}");
}

#[test]
fn the_layout_block_is_the_only_thing_positions_add() {
    let entities = vec![table("users", vec![key("id")]).with_position(Position::new(10.0, 20.0))];
    let plain = ddl(entities.clone());
    let positioned = positioned_ddl(entities);

    assert_eq!(
        positioned,
        format!("{plain}\n\n-- @schemaforge layout 1\n-- @table \"users\" 10 20")
    );
}

#[test]
fn an_empty_script_stays_empty_rather_than_becoming_a_layout_block() {
    assert_eq!(positioned_ddl(Vec::new()), "");
}

#[test]
fn the_layout_comes_after_the_descriptions() {
    let sql = PostgresGenerator.generate(
        &schema(vec![
            table("users", vec![key("id")]).with_description("Accounts")
        ]),
        options(true, true),
    );

    let comment = sql.find("COMMENT ON").expect("the comment is emitted");
    let layout = sql
        .find("@schemaforge layout")
        .expect("the layout is emitted");

    assert!(comment < layout, "got:\n{sql}");
}
