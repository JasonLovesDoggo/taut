//! Marker extraction from Python AST.
//!
//! Parses decorators like @skip, @mark, and @parallel from test functions.

use num_traits::cast::ToPrimitive;
use rustpython_parser::ast;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A marker attached to a test function.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Marker {
    /// Marker name (e.g., "skip", "mark", "parallel")
    pub name: String,
    /// Marker arguments
    pub args: MarkerArgs,
}

/// Arguments passed to a marker decorator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MarkerArgs {
    /// Positional argument (for @skip("reason"))
    pub reason: Option<String>,
    /// Keyword arguments (for @mark(slow=True, group="auth"))
    pub kwargs: HashMap<String, MarkerValue>,
}

/// A value in a marker argument.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MarkerValue {
    Bool(bool),
    String(String),
    Int(i64),
    Float(f64),
    List(Vec<String>),
}

impl std::fmt::Display for MarkerValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkerValue::Bool(b) => write!(f, "{}", b),
            MarkerValue::String(s) => write!(f, "{}", s),
            MarkerValue::Int(i) => write!(f, "{}", i),
            MarkerValue::Float(fl) => write!(f, "{}", fl),
            MarkerValue::List(items) => write!(f, "[{}]", items.join(", ")),
        }
    }
}

/// Extract markers from a function's decorator list.
pub fn extract_markers(decorators: &[ast::Expr]) -> Vec<Marker> {
    decorators.iter().filter_map(parse_decorator).collect()
}

/// Extract markers from a class's decorator list (for @parallel on class).
pub fn extract_class_markers(decorators: &[ast::Expr]) -> Vec<Marker> {
    decorators
        .iter()
        .filter_map(|d| {
            let marker = parse_decorator(d)?;
            // Only @parallel is valid on classes
            if marker.name == "parallel" {
                Some(marker)
            } else {
                None
            }
        })
        .collect()
}

/// Check if a test has the @skip marker.
pub fn is_skipped(markers: &[Marker]) -> bool {
    markers.iter().any(|m| m.name == "skip")
}

/// Get the skip reason if present.
pub fn get_skip_reason(markers: &[Marker]) -> Option<String> {
    markers
        .iter()
        .find(|m| m.name == "skip")
        .and_then(|m| m.args.reason.clone())
}

/// Check if a test has the @parallel marker.
pub fn is_parallel(markers: &[Marker]) -> bool {
    markers.iter().any(|m| m.name == "parallel")
}

/// Check if a test has @mark(slow=True).
pub fn is_slow(markers: &[Marker]) -> bool {
    markers.iter().any(|m| {
        m.name == "mark"
            && m.args
                .kwargs
                .get("slow")
                .map(|v| matches!(v, MarkerValue::Bool(true)))
                .unwrap_or(false)
    })
}

/// Get the group(s) from @mark(group="auth") or @mark(group=["a", "b"]).
pub fn get_groups(markers: &[Marker]) -> Vec<String> {
    markers
        .iter()
        .filter(|m| m.name == "mark")
        .filter_map(|m| m.args.kwargs.get("group"))
        .flat_map(|v| match v {
            MarkerValue::String(s) => vec![s.clone()],
            MarkerValue::List(items) => items.clone(),
            _ => vec![],
        })
        .collect()
}

/// Parse a single decorator expression into a Marker.
fn parse_decorator(decorator: &ast::Expr) -> Option<Marker> {
    match decorator {
        // @skip or @parallel (no parens)
        ast::Expr::Name(name) => {
            let name_str = name.id.as_str();
            if matches!(name_str, "skip" | "parallel") {
                Some(Marker {
                    name: name_str.to_string(),
                    args: MarkerArgs::default(),
                })
            } else {
                None
            }
        }

        // @skip("reason"), @mark(slow=True), @parallel()
        ast::Expr::Call(call) => parse_call_decorator(&call),

        // @taut.skip, @taut.parallel, etc. (attribute access)
        ast::Expr::Attribute(attr) => {
            let name_str = attr.attr.as_str();
            if matches!(name_str, "skip" | "parallel") {
                Some(Marker {
                    name: name_str.to_string(),
                    args: MarkerArgs::default(),
                })
            } else {
                None
            }
        }

        _ => None,
    }
}

/// Parse a call-style decorator like @skip("reason") or @mark(slow=True).
fn parse_call_decorator(call: &ast::ExprCall) -> Option<Marker> {
    let name = match call.func.as_ref() {
        ast::Expr::Name(name) => name.id.as_str().to_string(),
        ast::Expr::Attribute(attr) => attr.attr.as_str().to_string(),
        _ => return None,
    };

    if !matches!(name.as_str(), "skip" | "mark" | "parallel") {
        return None;
    }

    let mut args = MarkerArgs::default();

    // Parse positional arguments (mainly for @skip("reason"))
    if let Some(first_arg) = call.args.first() {
        if let Some(value) = expr_to_string(first_arg) {
            args.reason = Some(value);
        }
    }

    // Parse keyword arguments
    for keyword in &call.keywords {
        if let Some(ref arg_name) = keyword.arg {
            if let Some(value) = expr_to_marker_value(&keyword.value) {
                // Handle reason= for skip
                if arg_name.as_str() == "reason" {
                    if let MarkerValue::String(s) = &value {
                        args.reason = Some(s.clone());
                    }
                } else {
                    args.kwargs.insert(arg_name.to_string(), value);
                }
            }
        }
    }

    Some(Marker { name, args })
}

/// Convert an AST expression to a string (for @skip("reason")).
fn expr_to_string(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Str(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// Convert an AST expression to a MarkerValue.
fn expr_to_marker_value(expr: &ast::Expr) -> Option<MarkerValue> {
    match expr {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Bool(b) => Some(MarkerValue::Bool(*b)),
            ast::Constant::Str(s) => Some(MarkerValue::String(s.to_string())),
            ast::Constant::Int(i) => i.to_i64().map(MarkerValue::Int),
            ast::Constant::Float(f) => Some(MarkerValue::Float(*f)),
            _ => None,
        },

        // Handle lists: @mark(group=["auth", "integration"])
        ast::Expr::List(list) => {
            let items: Vec<String> = list.elts.iter().filter_map(expr_to_string).collect();
            if items.is_empty() {
                None
            } else {
                Some(MarkerValue::List(items))
            }
        }

        // Handle True/False as name expressions
        ast::Expr::Name(name) => match name.id.as_str() {
            "True" => Some(MarkerValue::Bool(true)),
            "False" => Some(MarkerValue::Bool(false)),
            _ => None,
        },

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpython_parser::Parse;

    fn parse_markers(code: &str) -> Vec<Marker> {
        let ast = ast::Suite::parse(code, "<test>").unwrap();
        for stmt in ast {
            if let ast::Stmt::FunctionDef(func) = stmt {
                return extract_markers(&func.decorator_list);
            }
        }
        vec![]
    }

    #[test]
    fn test_skip_no_args() {
        let markers = parse_markers(
            r#"
@skip
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "skip");
        assert!(markers[0].args.reason.is_none());
    }

    #[test]
    fn test_skip_with_reason() {
        let markers = parse_markers(
            r#"
@skip("API is down")
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "skip");
        assert_eq!(markers[0].args.reason, Some("API is down".to_string()));
    }

    #[test]
    fn test_skip_with_keyword_reason() {
        let markers = parse_markers(
            r#"
@skip(reason="Flaky test")
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "skip");
        assert_eq!(markers[0].args.reason, Some("Flaky test".to_string()));
    }

    #[test]
    fn test_mark_slow() {
        let markers = parse_markers(
            r#"
@mark(slow=True)
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "mark");
        assert_eq!(
            markers[0].args.kwargs.get("slow"),
            Some(&MarkerValue::Bool(true))
        );
        assert!(is_slow(&markers));
    }

    #[test]
    fn test_mark_group_string() {
        let markers = parse_markers(
            r#"
@mark(group="auth")
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].args.kwargs.get("group"),
            Some(&MarkerValue::String("auth".to_string()))
        );
        assert_eq!(get_groups(&markers), vec!["auth"]);
    }

    #[test]
    fn test_mark_group_list() {
        let markers = parse_markers(
            r#"
@mark(group=["auth", "integration"])
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].args.kwargs.get("group"),
            Some(&MarkerValue::List(vec![
                "auth".to_string(),
                "integration".to_string()
            ]))
        );
        assert_eq!(get_groups(&markers), vec!["auth", "integration"]);
    }

    #[test]
    fn test_mark_multiple_kwargs() {
        let markers = parse_markers(
            r#"
@mark(slow=True, group="integration")
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert!(is_slow(&markers));
        assert_eq!(get_groups(&markers), vec!["integration"]);
    }

    #[test]
    fn test_parallel_no_parens() {
        let markers = parse_markers(
            r#"
@parallel
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "parallel");
        assert!(is_parallel(&markers));
    }

    #[test]
    fn test_parallel_with_parens() {
        let markers = parse_markers(
            r#"
@parallel()
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "parallel");
        assert!(is_parallel(&markers));
    }

    #[test]
    fn test_multiple_markers() {
        let markers = parse_markers(
            r#"
@skip("broken")
@mark(slow=True)
@parallel
def test_foo():
    pass
"#,
        );
        assert_eq!(markers.len(), 3);
        assert!(is_skipped(&markers));
        assert!(is_slow(&markers));
        assert!(is_parallel(&markers));
    }

    #[test]
    fn test_unknown_decorator_ignored() {
        let markers = parse_markers(
            r#"
@pytest.mark.parametrize
@some_custom_decorator
def test_foo():
    pass
"#,
        );
        assert!(markers.is_empty());
    }
}
