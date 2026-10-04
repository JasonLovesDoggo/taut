//! Expand literal parameter sets during collection, without importing Python.
use anyhow::{Result, bail, ensure};
use num_traits::ToPrimitive;
use rustpython_parser::ast;
use serde_json::{Map, Number, Value};
use std::collections::{HashMap, HashSet};

use crate::discovery::TestItem;
use crate::markers::{Marker, MarkerArgs, MarkerValue};

const CASE_MARKER: &str = "parametrize-case";
const MAX_CASES: usize = 100_000;

struct Case {
    id: String,
    values: Map<String, Value>,
}

fn is_parametrize(expression: &ast::Expr) -> bool {
    match expression {
        ast::Expr::Name(name) => name.id.as_str() == "parametrize",
        ast::Expr::Attribute(attribute) => attribute.attr.as_str() == "parametrize",
        ast::Expr::Call(call) => is_parametrize(&call.func),
        _ => false,
    }
}

fn sequence(expression: &ast::Expr) -> Result<&[ast::Expr]> {
    match expression {
        ast::Expr::List(list) => Ok(&list.elts),
        ast::Expr::Tuple(tuple) => Ok(&tuple.elts),
        _ => {
            bail!("parametrize requires literal list or tuple rows; dynamic values are unsupported")
        }
    }
}

fn string(expression: &ast::Expr) -> Option<&str> {
    match expression {
        ast::Expr::Constant(constant) => match &constant.value {
            ast::Constant::Str(value) => Some(value),
            _ => None,
        },
        _ => None,
    }
}

fn parameter_names(expression: &ast::Expr) -> Result<Vec<String>> {
    let names = if let Some(value) = string(expression) {
        value.split(',').map(str::trim).map(str::to_owned).collect()
    } else {
        sequence(expression)?
            .iter()
            .map(|item| {
                string(item)
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("parameter names must be literal strings"))
            })
            .collect::<Result<Vec<_>>>()?
    };
    ensure!(
        !names.is_empty(),
        "parametrize requires at least one parameter name"
    );
    let mut seen = HashSet::new();
    for name in &names {
        let mut characters = name.chars();
        ensure!(
            characters
                .next()
                .is_some_and(|c| c == '_' || c.is_alphabetic())
                && characters.all(|c| c == '_' || c.is_alphanumeric()),
            "invalid parameter name {name:?}"
        );
        ensure!(seen.insert(name), "duplicate parameter name {name:?}");
    }
    Ok(names)
}

/// Parameter values must remain JSON values at invocation. Tuple containers are
/// valid row syntax, but tuple-valued arguments are rejected instead of silently
/// changing their Python type to list.
fn literal(expression: &ast::Expr) -> Result<Value> {
    match expression {
        ast::Expr::Constant(constant) => match &constant.value {
            ast::Constant::None => Ok(Value::Null),
            ast::Constant::Bool(value) => Ok(Value::Bool(*value)),
            ast::Constant::Str(value) => Ok(Value::String(value.clone())),
            ast::Constant::Int(value) => {
                let number = value
                    .to_i64()
                    .map(Number::from)
                    .or_else(|| value.to_u64().map(Number::from));
                number
                    .map(Value::Number)
                    .ok_or_else(|| anyhow::anyhow!("parameter integers must fit in 64 bits"))
            }
            ast::Constant::Float(value) => Number::from_f64(*value)
                .map(Value::Number)
                .ok_or_else(|| anyhow::anyhow!("parameter floats must be finite")),
            _ => bail!(
                "unsupported parameter literal; use JSON values (null, booleans, numbers, strings, lists, or string-keyed dictionaries)"
            ),
        },
        ast::Expr::UnaryOp(unary) => {
            let value = literal(&unary.operand)?;
            let Value::Number(number) = value else {
                bail!("parameter signs require a number")
            };
            match unary.op {
                ast::UnaryOp::UAdd => Ok(Value::Number(number)),
                ast::UnaryOp::USub => {
                    if let Some(integer) = number.as_i64() {
                        integer
                            .checked_neg()
                            .map(|value| Value::Number(value.into()))
                            .ok_or_else(|| {
                                anyhow::anyhow!("parameter integers must fit in 64 bits")
                            })
                    } else if let Some(integer) = number.as_u64() {
                        ensure!(
                            integer == (i64::MAX as u64) + 1,
                            "parameter integers must fit in 64 bits"
                        );
                        Ok(Value::Number(i64::MIN.into()))
                    } else {
                        Ok(Value::Number(
                            Number::from_f64(-number.as_f64().unwrap()).unwrap(),
                        ))
                    }
                }
                _ => bail!("only literal numeric signs are supported in parameter values"),
            }
        }
        ast::Expr::List(list) => list
            .elts
            .iter()
            .map(literal)
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        ast::Expr::Dict(dict) => {
            let mut values = Map::new();
            for (key, value) in dict.keys.iter().zip(&dict.values) {
                let key = key.as_ref().and_then(string)
                    .ok_or_else(|| anyhow::anyhow!("parameter dictionaries require literal string keys; unpacking is unsupported"))?;
                ensure!(
                    !values.contains_key(key),
                    "duplicate parameter dictionary key {key:?}"
                );
                values.insert(key.to_owned(), literal(value)?);
            }
            Ok(Value::Object(values))
        }
        ast::Expr::Tuple(_) => bail!(
            "tuple-valued parameters are unsupported; use a list to preserve JSON value semantics (tuple rows are supported)"
        ),
        _ => bail!("dynamic parameter values are unsupported; use literal JSON values"),
    }
}

// Escape separators and control characters so printed IDs are safe to copy into
// exact positional selectors and -k patterns. Preserve readable ASCII labels.
fn escape_id(id: &str) -> String {
    if id.is_empty() {
        return "empty".to_owned();
    }
    let mut result = String::new();
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn scalar_id(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("None".to_owned()),
        Value::Bool(true) => Some("True".to_owned()),
        Value::Bool(false) => Some("False".to_owned()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn cases(call: &ast::ExprCall) -> Result<Vec<Case>> {
    ensure!(
        call.args.len() <= 2,
        "parametrize accepts two positional arguments: names and values"
    );
    let mut names = call.args.first();
    let mut rows = call.args.get(1);
    let mut ids = None;
    let mut seen_keywords = HashSet::new();
    for keyword in &call.keywords {
        let name = keyword
            .arg
            .as_ref()
            .map(|name| name.as_str())
            .ok_or_else(|| anyhow::anyhow!("parametrize keyword unpacking is unsupported"))?;
        ensure!(
            seen_keywords.insert(name),
            "duplicate parametrize keyword {name:?}"
        );
        match name {
            "argnames" => {
                ensure!(names.is_none(), "parametrize names supplied twice");
                names = Some(&keyword.value);
            }
            "argvalues" => {
                ensure!(rows.is_none(), "parametrize values supplied twice");
                rows = Some(&keyword.value);
            }
            "ids" => ids = Some(&keyword.value),
            "indirect" => ensure!(
                literal(&keyword.value)? == Value::Bool(false),
                "indirect parametrization is unsupported"
            ),
            _ => bail!(
                "unsupported parametrize option {name:?}; supported options are argnames, argvalues, and literal ids"
            ),
        }
    }
    let names_expression =
        names.ok_or_else(|| anyhow::anyhow!("parametrize requires parameter names"))?;
    let names = parameter_names(names_expression)?;
    // pytest treats a string name as a scalar value; ['name'] still uses rows.
    let scalar_rows = names.len() == 1 && string(names_expression).is_some();
    let rows =
        sequence(rows.ok_or_else(|| anyhow::anyhow!("parametrize requires parameter values"))?)?;
    ensure!(
        !rows.is_empty(),
        "empty parameter set: no test cases would run"
    );
    ensure!(
        rows.len() <= MAX_CASES,
        "parameter set exceeds {MAX_CASES} cases"
    );
    let ids = ids.filter(|expression| !matches!(expression, ast::Expr::Constant(constant) if constant.value == ast::Constant::None))
        .map(sequence).transpose()?;
    if let Some(ids) = ids {
        ensure!(
            ids.len() == rows.len(),
            "parametrize ids count ({}) must match row count ({})",
            ids.len(),
            rows.len()
        );
    }
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let row_values = if scalar_rows {
                vec![literal(row)?]
            } else {
                let row = sequence(row)?;
                ensure!(
                    row.len() == names.len(),
                    "parameter row {} has {} values for {} names",
                    index + 1,
                    row.len(),
                    names.len()
                );
                row.iter().map(literal).collect::<Result<Vec<_>>>()?
            };
            let default_id = names
                .iter()
                .zip(&row_values)
                .map(|(name, value)| scalar_id(value).unwrap_or_else(|| format!("{name}{index}")))
                .collect::<Vec<_>>()
                .join("-");
            let id = if let Some(ids) = ids {
                let value = literal(&ids[index])?;
                if value.is_null() {
                    default_id
                } else {
                    scalar_id(&value).ok_or_else(|| {
                        anyhow::anyhow!(
                            "parameter ids must be literal strings, numbers, booleans, or None"
                        )
                    })?
                }
            } else {
                default_id
            };
            Ok(Case {
                id: escape_id(&id),
                values: names.iter().cloned().zip(row_values).collect(),
            })
        })
        .collect()
}

fn unique_ids(cases: &mut [Case]) {
    let mut counts = HashMap::new();
    let mut used = HashSet::new();
    for case in cases.iter() {
        *counts.entry(case.id.clone()).or_insert(0) += 1;
        used.insert(case.id.clone());
    }
    let mut suffixes = HashMap::new();
    for case in cases {
        if counts[&case.id] > 1 {
            let next = suffixes.entry(case.id.clone()).or_insert(0);
            loop {
                let candidate = format!("{}-{next}", case.id);
                *next += 1;
                if used.insert(candidate.clone()) {
                    case.id = candidate;
                    break;
                }
            }
        }
    }
}

pub fn expand_into<'a>(
    item: TestItem,
    decorators: impl DoubleEndedIterator<Item = &'a ast::Expr>,
    output: &mut Vec<TestItem>,
) -> Result<()> {
    let mut decorators = decorators
        .rev()
        .filter(|decorator| is_parametrize(decorator))
        .peekable();
    // Ordinary tests pay no parameter-related allocations or AST clones.
    if decorators.peek().is_none() {
        output.push(item);
        return Ok(());
    }
    let mut combined = vec![Case {
        id: String::new(),
        values: Map::new(),
    }];
    for decorator in decorators {
        let ast::Expr::Call(call) = decorator else {
            bail!("parametrize must be called with literal names and values")
        };
        let rows = cases(call)?;
        ensure!(
            combined
                .len()
                .checked_mul(rows.len())
                .is_some_and(|count| count <= MAX_CASES),
            "Cartesian parameter set exceeds {MAX_CASES} cases"
        );
        let mut next = Vec::with_capacity(combined.len() * rows.len());
        for previous in combined {
            for row in &rows {
                let mut values = previous.values.clone();
                for (name, value) in &row.values {
                    ensure!(
                        !values.contains_key(name),
                        "parameter {name:?} is repeated in stacked decorators"
                    );
                    values.insert(name.clone(), value.clone());
                }
                let id = if previous.id.is_empty() {
                    row.id.clone()
                } else {
                    format!("{}-{}", previous.id, row.id)
                };
                next.push(Case { id, values });
            }
        }
        combined = next;
    }
    unique_ids(&mut combined);
    output.extend(combined.into_iter().map(|case| {
        let mut expanded = item.clone();
        expanded.markers.push(Marker {
            name: CASE_MARKER.to_owned(),
            args: MarkerArgs {
                reason: None,
                kwargs: HashMap::from([
                    ("id".to_owned(), MarkerValue::String(case.id)),
                    (
                        "values".to_owned(),
                        MarkerValue::String(Value::Object(case.values).to_string()),
                    ),
                ]),
            },
        });
        expanded
    }));
    Ok(())
}

fn field<'a>(item: &'a TestItem, name: &str) -> Option<&'a str> {
    match item
        .markers
        .iter()
        .find(|marker| marker.name == CASE_MARKER)?
        .args
        .kwargs
        .get(name)?
    {
        MarkerValue::String(value) => Some(value),
        _ => None,
    }
}

pub fn case_id(item: &TestItem) -> Option<&str> {
    field(item, "id")
}

pub fn parameters(item: &TestItem) -> Option<Value> {
    field(item, "values").and_then(|value| serde_json::from_str(value).ok())
}
