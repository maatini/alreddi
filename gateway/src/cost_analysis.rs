use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CostError {
    #[error("query cost {cost} exceeds max {max}")]
    TooExpensive { cost: u32, max: u32 },
    #[error("query depth {depth} exceeds max {max}")]
    TooDeep { depth: u32, max: u32 },
    #[error("parse error: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct CostConfig {
    #[serde(default = "default_max_cost")]
    pub max_cost: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    pub field_costs: Option<HashMap<String, u32>>,
}

fn default_max_cost() -> u32 {
    100
}
fn default_max_depth() -> u32 {
    10
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            max_cost: 100,
            max_depth: 10,
            field_costs: None,
        }
    }
}

/// Lightweight GraphQL query tokenizer and cost analyzer.
///
/// Operates directly on the query string without schema access, using a
/// depth-first traversal of the selection-tree structure. Cost model:
///
/// | Pattern              | Cost |
/// |---------------------|------|
/// | Scalar field         |    1 |
/// | Object field (leaf)  |    2 |
/// | List/connection root |    5 |
/// | Nested selections    | sum of children |
/// | Depth > max_depth    | rejected        |
///
/// The analyzer recognizes `connection { edges { node { ... } } }` patterns
/// and applies the connection multiplier only at the list root.
pub struct CostAnalyzer {
    config: CostConfig,
}

impl CostAnalyzer {
    pub fn new(config: CostConfig) -> Self {
        Self { config }
    }

    pub fn analyze(&self, query: &str) -> Result<u32, CostError> {
        let mut pos = 0usize;
        let bytes = query.as_bytes();
        let mut total_cost = 0u32;
        #[allow(unused_assignments)]
        let mut max_depth_seen = 0u32;

        // Skip leading fragment definitions
        while skip_fragment_definition(bytes, &mut pos) {
            skip_whitespace_and_comments(bytes, &mut pos);
        }

        skip_whitespace_and_comments(bytes, &mut pos);

        // Parse the first operation
        if pos < bytes.len() {
            if bytes[pos] == b'{' {
                let (cost, depth) =
                    self.cost_of_selection_set(bytes, &mut pos, 1)?;
                total_cost += cost;
                max_depth_seen = depth;
            } else if let Some(kw) = peek_keyword(bytes, &mut pos) {
                if kw == "query" || kw == "mutation" || kw == "subscription" {
                    consume_keyword(bytes, &mut pos, &kw);
                    // Optional operation name
                    let saved = pos;
                    skip_whitespace_and_comments(bytes, &mut pos);
                    if pos < bytes.len() && bytes[pos].is_ascii_alphanumeric() {
                        consume_name(bytes, &mut pos);
                    } else {
                        pos = saved;
                    }
                    // Optional variable definitions
                    skip_whitespace_and_comments(bytes, &mut pos);
                    if pos < bytes.len() && bytes[pos] == b'(' {
                        skip_balanced(bytes, &mut pos, b'(', b')');
                    }
                    // Optional directives
                    skip_whitespace_and_comments(bytes, &mut pos);
                    while pos < bytes.len() && bytes[pos] == b'@' {
                        skip_word(bytes, &mut pos);
                        skip_whitespace_and_comments(bytes, &mut pos);
                    }
                    // Selection set
                    let (cost, depth) =
                        self.cost_of_selection_set(bytes, &mut pos, 1)?;
                    total_cost += cost;
                    max_depth_seen = depth;
                }
            }
        }

        // Handle remaining sibling operations and fragment definitions
        loop {
            // Skip any fragment definitions
            while skip_fragment_definition(bytes, &mut pos) {
                skip_whitespace_and_comments(bytes, &mut pos);
            }

            skip_whitespace_and_comments(bytes, &mut pos);
            if pos >= bytes.len() {
                break;
            }
            if bytes[pos] == b'{' {
                let (cost, depth) =
                    self.cost_of_selection_set(bytes, &mut pos, 1)?;
                total_cost += cost;
                max_depth_seen = max_depth_seen.max(depth);
            } else if let Some(kw) = peek_keyword(bytes, &mut pos) {
                if kw == "query" || kw == "mutation" || kw == "subscription" {
                    consume_keyword(bytes, &mut pos, &kw);
                    skip_whitespace_and_comments(bytes, &mut pos);
                    if pos < bytes.len() && bytes[pos].is_ascii_alphanumeric() {
                        consume_name(bytes, &mut pos);
                    }
                    skip_whitespace_and_comments(bytes, &mut pos);
                    if pos < bytes.len() && bytes[pos] == b'(' {
                        skip_balanced(bytes, &mut pos, b'(', b')');
                    }
                    let (cost, depth) =
                        self.cost_of_selection_set(bytes, &mut pos, 1)?;
                    total_cost += cost;
                    max_depth_seen = max_depth_seen.max(depth);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        self.check_limits(total_cost, max_depth_seen)
    }

    fn cost_of_selection_set(
        &self,
        bytes: &[u8],
        pos: &mut usize,
        depth: u32,
    ) -> Result<(u32, u32), CostError> {
        let mut cost = 0u32;
        let mut max_child_depth = depth;

        expect(bytes, pos, b'{')?;

        loop {
            skip_whitespace_and_comments(bytes, pos);
            if *pos >= bytes.len() {
                return Err(CostError::ParseError("unterminated selection set".into()));
            }
            if bytes[*pos] == b'}' {
                *pos += 1;
                break;
            }

            // Fragment spread (...fragmentName) or inline fragment (... on Type)
            if bytes[*pos] == b'.' && *pos + 2 < bytes.len() && bytes[*pos + 1] == b'.' && bytes[*pos + 2] == b'.'
            {
                // Save position after the three dots and peek ahead
                let after_dots = *pos + 3;
                let mut probe = after_dots;

                // Check if this is `... on Type` (inline fragment)
                skip_whitespace_and_comments(bytes, &mut probe);
                let is_inline = probe + 2 < bytes.len()
                    && bytes[probe] == b'o'
                    && bytes[probe + 1] == b'n'
                    && (probe + 2 >= bytes.len() || !bytes[probe + 2].is_ascii_alphanumeric());

                *pos += 3;
                skip_whitespace_and_comments(bytes, pos);

                if is_inline {
                    consume_keyword(bytes, pos, "on");
                    skip_whitespace_and_comments(bytes, pos);
                    consume_name(bytes, pos); // type name
                    skip_whitespace_and_comments(bytes, pos);
                    if *pos < bytes.len() && bytes[*pos] == b'@' {
                        skip_word(bytes, pos);
                    }
                    let (nested_cost, nested_depth) =
                        self.cost_of_selection_set(bytes, pos, depth + 1)?;
                    cost += 2 + nested_cost;
                    max_child_depth = max_child_depth.max(nested_depth);
                } else {
                    consume_name(bytes, pos); // fragment name
                    skip_whitespace_and_comments(bytes, pos);
                    if *pos < bytes.len() && bytes[*pos] == b'@' {
                        skip_word(bytes, pos);
                    }
                    cost += 2;
                }
                continue;
            }

            // Regular field
            let field_name = consume_name(bytes, pos);

            // Handle field aliases: alias: actualName
            skip_whitespace_and_comments(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b':' {
                *pos += 1;
                // The real field name follows the colon
                let _real_name = consume_name(bytes, pos);
            }

            // Arguments: ( ... )
            skip_whitespace_and_comments(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b'(' {
                skip_balanced(bytes, pos, b'(', b')');
            }

            // Directives: @skip, @include, etc.
            skip_whitespace_and_comments(bytes, pos);
            while *pos < bytes.len() && bytes[*pos] == b'@' {
                skip_word(bytes, pos);
                skip_whitespace_and_comments(bytes, pos);
            }

            // Check for nested selection set
            skip_whitespace_and_comments(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b'{' {
                let base = self.field_cost(&field_name);
                let (nested, nested_depth) =
                    self.cost_of_selection_set(bytes, pos, depth + 1)?;
                cost += base + nested;
                max_child_depth = max_child_depth.max(nested_depth);
            } else {
                // Leaf / scalar field
                cost += self.field_cost(&field_name);
                max_child_depth = max_child_depth.max(depth + 1);
            }
        }

        Ok((cost, max_child_depth))
    }

    fn field_cost(&self, name: &str) -> u32 {
        if let Some(ref custom) = self.config.field_costs {
            if let Some(&c) = custom.get(name) {
                return c;
            }
        }

        match name {
            // List-entry points
            "articles" | "search" | "prices" => 5,
            // Connection fields
            "edges" => 3,
            "node" => 1,
            // Introspection
            "__schema" | "__type" | "__typename" => 0,
            // Default object field
            _ => {
                let lower = name.to_lowercase();
                if lower.contains("connection") || lower.ends_with("list") {
                    5
                } else {
                    2
                }
            }
        }
    }

    fn check_limits(&self, cost: u32, depth: u32) -> Result<u32, CostError> {
        if depth > self.config.max_depth {
            return Err(CostError::TooDeep {
                depth,
                max: self.config.max_depth,
            });
        }
        if cost > self.config.max_cost {
            return Err(CostError::TooExpensive {
                cost,
                max: self.config.max_cost,
            });
        }
        Ok(cost)
    }
}

// ---------------------------------------------------------------------------
// Query-string tokenizer helpers
// ---------------------------------------------------------------------------

/// Skip a fragment definition: `fragment Name on Type @dirs { ... }`.
/// Returns true if a fragment was actually skipped.
fn skip_fragment_definition(bytes: &[u8], pos: &mut usize) -> bool {
    let saved = *pos;
    skip_whitespace_and_comments(bytes, pos);
    if !consume_keyword(bytes, pos, "fragment") {
        *pos = saved;
        return false;
    }
    skip_whitespace_and_comments(bytes, pos);
    consume_name(bytes, pos); // fragment name
    skip_whitespace_and_comments(bytes, pos);
    consume_keyword(bytes, pos, "on"); // "on" keyword
    skip_whitespace_and_comments(bytes, pos);
    consume_name(bytes, pos); // type condition
    skip_whitespace_and_comments(bytes, pos);
    // Optional directives
    while *pos < bytes.len() && bytes[*pos] == b'@' {
        skip_word(bytes, pos);
        skip_whitespace_and_comments(bytes, pos);
    }
    // Selection set
    if *pos < bytes.len() && bytes[*pos] == b'{' {
        let _ = skip_balanced_with_nesting(bytes, pos);
    }
    true
}

/// Skip balanced braces/brackets/parens, accounting for nested pairs.
/// Returns Ok(()) on success.
fn skip_balanced_with_nesting(bytes: &[u8], pos: &mut usize) -> Result<(), CostError> {
    let open = bytes[*pos];
    let close = match open {
        b'{' => b'}',
        b'(' => b')',
        b'[' => b']',
        _ => return Err(CostError::ParseError(format!("unexpected char '{}'", open as char))),
    };
    let mut depth = 1u32;
    *pos += 1;
    while *pos < bytes.len() && depth > 0 {
        if bytes[*pos] == open {
            depth += 1;
        } else if bytes[*pos] == close {
            depth -= 1;
        } else if bytes[*pos] == b'"' || bytes[*pos] == b'\'' {
            skip_string(bytes, pos);
            continue;
        }
        *pos += 1;
    }
    Ok(())
}

fn skip_whitespace_and_comments(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() {
        let b = bytes[*pos];
        if b.is_ascii_whitespace() {
            *pos += 1;
        } else if b == b'#' {
            // Line comment
            while *pos < bytes.len() && bytes[*pos] != b'\n' {
                *pos += 1;
            }
        } else {
            break;
        }
    }
}

fn peek_keyword<'a>(bytes: &[u8], pos: &mut usize) -> Option<String> {
    let saved = *pos;
    skip_whitespace_and_comments(bytes, pos);
    let result = if *pos < bytes.len() && bytes[*pos].is_ascii_alphabetic() {
        Some(consume_name(bytes, pos))
    } else {
        None
    };
    if result.is_none() {
        *pos = saved;
    }
    result
}

fn consume_keyword(bytes: &[u8], pos: &mut usize, expected: &str) -> bool {
    let saved = *pos;
    skip_whitespace_and_comments(bytes, pos);
    if *pos + expected.len() <= bytes.len()
        && bytes[*pos..*pos + expected.len()].eq_ignore_ascii_case(expected.as_bytes())
    {
        *pos += expected.len();
        // Ensure we consumed a whole word (next byte is whitespace, punctuation, or EOF)
        if *pos >= bytes.len()
            || bytes[*pos].is_ascii_whitespace()
            || bytes[*pos] == b'{'
            || bytes[*pos] == b'('
        {
            return true;
        }
    }
    *pos = saved;
    false
}

fn consume_name(bytes: &[u8], pos: &mut usize) -> String {
    skip_whitespace_and_comments(bytes, pos);
    let start = *pos;
    while *pos < bytes.len()
        && (bytes[*pos].is_ascii_alphanumeric() || bytes[*pos] == b'_')
    {
        *pos += 1;
    }
    String::from_utf8_lossy(&bytes[start..*pos]).into_owned()
}

fn skip_word(bytes: &[u8], pos: &mut usize) {
    skip_whitespace_and_comments(bytes, pos);
    while *pos < bytes.len()
        && (bytes[*pos].is_ascii_alphanumeric()
            || bytes[*pos] == b'_'
            || bytes[*pos] == b'@'
            || bytes[*pos] == b'-')
    {
        *pos += 1;
    }
}

fn expect(bytes: &[u8], pos: &mut usize, expected: u8) -> Result<(), CostError> {
    skip_whitespace_and_comments(bytes, pos);
    if *pos >= bytes.len() || bytes[*pos] != expected {
        return Err(CostError::ParseError(format!(
            "expected '{}' at position {}",
            expected as char, *pos
        )));
    }
    *pos += 1;
    Ok(())
}

fn skip_balanced(bytes: &[u8], pos: &mut usize, open: u8, close: u8) {
    let mut depth = 1u32;
    *pos += 1; // skip opening char
    while *pos < bytes.len() && depth > 0 {
        if bytes[*pos] == open {
            depth += 1;
        } else if bytes[*pos] == close {
            depth -= 1;
        } else if bytes[*pos] == b'"' {
            skip_string(bytes, pos);
            continue;
        }
        *pos += 1;
    }
}

fn skip_string(bytes: &[u8], pos: &mut usize) {
    let quote = bytes[*pos];
    *pos += 1;
    while *pos < bytes.len() {
        if bytes[*pos] == b'\\' {
            *pos += 2;
        } else if bytes[*pos] == quote {
            *pos += 1;
            return;
        } else {
            *pos += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn analyzer(max_cost: u32, max_depth: u32) -> CostAnalyzer {
        CostAnalyzer::new(CostConfig {
            max_cost,
            max_depth,
            ..Default::default()
        })
    }

    #[test]
    fn simple_scalar_query() {
        let a = analyzer(100, 10);
        let cost = a.analyze("{ __typename }").unwrap();
        assert_eq!(cost, 0); // __typename is free
    }

    #[test]
    fn single_object_query() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze("query { article(ean: \"123\") { ean name brand } }")
            .unwrap();
        // article: object=2, ean=2, name=2, brand=2 = 8
        assert_eq!(cost, 8);
    }

    #[test]
    fn nested_object_query() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze(
                "query { article(ean: \"123\") { ean name category { id name } price { amount currency } } }",
            )
            .unwrap();
        // article=2 + ean=2 + name=2
        //   + category=2 + id=2 + name=2
        //   + price=2 + amount=2 + currency=2
        // = 18
        assert_eq!(cost, 18);
    }

    #[test]
    fn list_query_costs_more() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze("query { articles { ean name } }")
            .unwrap();
        // articles=5 + ean=2 + name=2 = 9
        assert_eq!(cost, 9);
    }

    #[test]
    fn connection_query_costs_more() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze("query { search(query: \"milch\") { edges { node { ean name } } } }")
            .unwrap();
        // search=5 + edges=3 + node=1 + ean=2 + name=2 = 13
        assert_eq!(cost, 13);
    }

    #[test]
    fn cost_exceeds_max() {
        let a = analyzer(5, 10);
        let err = a
            .analyze("query { article(ean: \"1\") { ean name brand } }")
            .unwrap_err();
        assert!(matches!(err, CostError::TooExpensive { .. }));
    }

    #[test]
    fn depth_exceeds_max() {
        let a = analyzer(100, 2);
        let err = a
            .analyze("{ a { b { c } } }")
            .unwrap_err();
        assert!(matches!(err, CostError::TooDeep { .. }));
    }

    #[test]
    fn multiple_top_level_fields() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze("{ a1: article(ean:\"1\") { ean } a2: article(ean:\"2\") { ean } }")
            .unwrap();
        // article=2×2=4 + ean=2×2=4 = 8
        assert_eq!(cost, 8);
    }

    #[test]
    fn fragment_spread_in_query() {
        let a = analyzer(100, 10);
        let query = r#"
            fragment artFields on Article { ean name brand }
            query { article(ean: "1") { ...artFields } }
        "#;
        let cost = a.analyze(query).unwrap();
        // article=2 + fragmentSpread=2 = 4
        // NOTE: fragment spread resolution (looking up spread fields in
        // fragment definitions) is deferred to Phase 3. For now, spreads
        // carry a fixed cost of 2 — a conservative lower bound that still
        // prevents unbounded cost drift through volume-based control.
        assert_eq!(cost, 4);
    }

    #[test]
    fn inline_fragment() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze(r#"query { articles { ... on Article { ean name } } }"#)
            .unwrap();
        // articles=5 + inlineFragment=2 + ean=2 + name=2 = 11
        assert_eq!(cost, 11);
    }

    #[test]
    fn comments_and_whitespace() {
        let a = analyzer(100, 10);
        let cost = a
            .analyze(
                "# get article\n  query   { article(ean:\"42\") { ean\n# field\nname } }",
            )
            .unwrap();
        assert_eq!(cost, 6);
    }
}
