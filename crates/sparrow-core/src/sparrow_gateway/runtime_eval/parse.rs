use crate::sparrowc::{
    analyzer::analyze,
    parser::{
        SparrowParser,
        types::{Content, HxFile, Source},
    },
};
use super::RuntimeError;

#[cfg(test)]
const TEST_SCHEMA: &str = r#"
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    age: I32
}
"#;

pub fn parse_and_validate(
    schema_hql: &str,
    query_hql: &str,
) -> Result<Source, RuntimeError> {
    let content = Content {
        content: format!("{schema_hql}\n{query_hql}"),
        source: Source::default(),
        files: vec![
            HxFile { name: "schema.hx".to_string(), content: schema_hql.to_string() },
            HxFile { name: "runtime.hx".to_string(), content: query_hql.to_string() },
        ],
    };

    let source = SparrowParser::parse_source(&content)
        .map_err(|e| RuntimeError::Parse(e.to_string()))?;

    let (diagnostics, _) = analyze(&source)
        .map_err(|e| RuntimeError::Analysis(e.to_string()))?;

    if !diagnostics.is_empty() {
        let msgs: Vec<String> = diagnostics.iter()
            .map(|d| d.message.clone())
            .collect();
        return Err(RuntimeError::Analysis(msgs.join("; ")));
    }

    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_query_parses_ok() {
        let query = "QUERY getAll() =>\n    people <- N<People>\nRETURN people";
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let source = result.unwrap();
        assert_eq!(source.queries.len(), 1);
        assert_eq!(source.queries[0].name, "getAll");
    }

    #[test]
    fn test_unknown_type_fails_analysis() {
        let query = "QUERY bad() =>\n    x <- N<Nonexistent>\nRETURN x";
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_err(), "expected Err for unknown type");
    }

    #[test]
    fn test_syntax_error_fails_parse() {
        let query = "QUERY bad() => @@@ RETURN x";
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_err(), "expected Err for syntax error");
    }

    #[test]
    fn test_query_with_param() {
        let query = "QUERY getPerson(person_id: String) =>\n    person <- N<People>({person_id: person_id})\nRETURN person";
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    }
}
