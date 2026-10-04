//! Import-free class collection. Resolve local bases and the standard unittest
//! roots, then use Python's C3 ordering and class attribute shadowing rules.
use super::{LineIndex, TestItem, is_test_name, markers, reject_compound_tests};
use anyhow::{Context, Result, bail};
use rustpython_parser::ast::{self, Ranged};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ClassKey {
    Local(usize),
    Object,
    TestCase,
    AsyncTestCase,
}

#[derive(Clone)]
enum Symbol {
    Class(ClassKey),
    Qualified(String),
    UncertainClass { offset: usize, unittest: bool },
    Unknown,
}

struct Binding<'a> {
    name: &'a str,
    definition: Option<&'a ast::Stmt>,
    statement: &'a ast::Stmt,
}

struct Class<'a> {
    node: &'a ast::StmtClassDef,
    mro: std::result::Result<Vec<ClassKey>, String>,
    unittest: bool,
    bindings: Vec<Binding<'a>>,
}

pub(super) struct Classes<'a> {
    symbols: HashMap<String, Symbol>,
    classes: Vec<Class<'a>>,
    conditional_tests: Vec<(String, usize)>,
}

impl<'a> Classes<'a> {
    pub(super) fn new(suite: &'a [ast::Stmt]) -> Self {
        let mut result = Self {
            symbols: HashMap::from([("object".to_owned(), Symbol::Class(ClassKey::Object))]),
            classes: Vec::new(),
            conditional_tests: Vec::new(),
        };
        for stmt in suite {
            match stmt {
                ast::Stmt::ClassDef(node) => {
                    let index = result.classes.len();
                    let bases: Vec<_> = node
                        .bases
                        .iter()
                        .map(|base| result.resolve_base(base))
                        .collect();
                    let unittest = bases
                        .iter()
                        .filter_map(|base| base.as_ref().ok())
                        .any(|base| match base {
                            ClassKey::TestCase | ClassKey::AsyncTestCase => true,
                            ClassKey::Local(index) => result.classes[*index].unittest,
                            ClassKey::Object => false,
                        });
                    let mro = result.linearize(index, &bases);
                    result.classes.push(Class {
                        node,
                        mro,
                        unittest,
                        bindings: class_bindings(&node.body),
                    });
                    result
                        .symbols
                        .insert(node.name.to_string(), Symbol::Class(ClassKey::Local(index)));
                }
                ast::Stmt::Import(import) => {
                    for alias in &import.names {
                        let (name, qualified) = match &alias.asname {
                            Some(name) => (name.as_str(), alias.name.as_str()),
                            None => {
                                let root = alias
                                    .name
                                    .as_str()
                                    .split('.')
                                    .next()
                                    .unwrap_or(alias.name.as_str());
                                (root, root)
                            }
                        };
                        result
                            .symbols
                            .insert(name.to_owned(), Symbol::Qualified(qualified.to_owned()));
                    }
                }
                ast::Stmt::ImportFrom(import) => {
                    let module = import.module.as_ref().map_or("", |name| name.as_str());
                    let prefix =
                        ".".repeat(import.level.map_or(0, |level| level.to_u32() as usize));
                    for alias in &import.names {
                        result.symbols.insert(
                            alias.asname.as_ref().unwrap_or(&alias.name).to_string(),
                            Symbol::Qualified(format!("{prefix}{module}.{}", alias.name)),
                        );
                    }
                }
                ast::Stmt::Assign(assign) => {
                    let symbol = result.resolve_symbol(&assign.value);
                    for target in &assign.targets {
                        for name in target_names(target) {
                            result.symbols.insert(name.to_owned(), symbol.clone());
                        }
                    }
                }
                ast::Stmt::AnnAssign(assign) => {
                    if let Some(value) = &assign.value {
                        let symbol = result.resolve_symbol(value);
                        for name in target_names(&assign.target) {
                            result.symbols.insert(name.to_owned(), symbol.clone());
                        }
                    }
                }
                ast::Stmt::FunctionDef(function) => {
                    result
                        .symbols
                        .insert(function.name.to_string(), Symbol::Unknown);
                }
                ast::Stmt::AsyncFunctionDef(function) => {
                    result
                        .symbols
                        .insert(function.name.to_string(), Symbol::Unknown);
                }
                _ => {
                    let mut conditional_tests = Vec::new();
                    visit_scope(stmt, &mut |statement| {
                        if let ast::Stmt::ClassDef(class) = statement {
                            let unittest =
                                class
                                    .bases
                                    .iter()
                                    .any(|base| match result.resolve_base(base) {
                                        Ok(ClassKey::TestCase | ClassKey::AsyncTestCase) => true,
                                        Ok(ClassKey::Local(index)) => {
                                            result.classes[index].unittest
                                        }
                                        _ => false,
                                    });
                            if class.name.as_str().starts_with("Test") || unittest {
                                conditional_tests.push((
                                    class.name.to_string(),
                                    usize::from(class.range.start()),
                                ));
                            }
                        }
                    });
                    result.conditional_tests.extend(conditional_tests);
                    // Conditional module rebinding makes a previously known base
                    // uncertain. Reject uses of it instead of guessing a branch.
                    for name in statement_names(stmt) {
                        let symbol = match result.symbols.get(name) {
                            Some(Symbol::Class(ClassKey::Local(index))) => Symbol::UncertainClass {
                                offset: stmt.range().start().into(),
                                unittest: result.classes[*index].unittest,
                            },
                            Some(symbol @ Symbol::UncertainClass { .. }) => symbol.clone(),
                            _ => Symbol::Unknown,
                        };
                        result.symbols.insert(name.to_owned(), symbol);
                    }
                }
            }
        }
        result
    }

    pub(super) fn needed(suite: &[ast::Stmt]) -> bool {
        suite.iter().any(|stmt| {
            matches!(
                stmt,
                ast::Stmt::ClassDef(_)
                    | ast::Stmt::If(_)
                    | ast::Stmt::For(_)
                    | ast::Stmt::AsyncFor(_)
                    | ast::Stmt::While(_)
                    | ast::Stmt::With(_)
                    | ast::Stmt::AsyncWith(_)
                    | ast::Stmt::Try(_)
                    | ast::Stmt::TryStar(_)
                    | ast::Stmt::Match(_)
            )
        })
    }

    pub(super) fn reject_conditional_classes(&self, path: &Path, lines: &LineIndex) -> Result<()> {
        if let Some((name, offset)) = self.conditional_tests.first() {
            bail!(
                "Cannot collect {}:{}: test class '{}' defined inside conditional or compound statements is unsupported; define test classes at module scope",
                path.display(),
                lines.line(*offset),
                name
            );
        }
        if let Some((name, offset)) = self
            .symbols
            .iter()
            .filter_map(|(name, symbol)| match symbol {
                Symbol::UncertainClass { offset, unittest }
                    if name.starts_with("Test") || *unittest =>
                {
                    Some((name, *offset))
                }
                _ => None,
            })
            .min_by_key(|(name, offset)| (*offset, *name))
        {
            bail!(
                "Cannot collect {}:{}: conditional rebinding of test class '{}' is unsupported; keep its final binding unconditional",
                path.display(),
                lines.line(offset),
                name
            );
        }
        Ok(())
    }

    fn resolve_symbol(&self, expr: &ast::Expr) -> Symbol {
        match expr {
            ast::Expr::Name(name) => self
                .symbols
                .get(name.id.as_str())
                .cloned()
                .unwrap_or(Symbol::Unknown),
            ast::Expr::Attribute(attribute) => match self.resolve_symbol(&attribute.value) {
                Symbol::Qualified(module) => {
                    Symbol::Qualified(format!("{module}.{}", attribute.attr))
                }
                _ => Symbol::Unknown,
            },
            _ => Symbol::Unknown,
        }
    }

    fn resolve_base(&self, expr: &ast::Expr) -> std::result::Result<ClassKey, String> {
        match self.resolve_symbol(expr) {
            Symbol::Class(class) => Ok(class),
            Symbol::Qualified(name) => match name.as_str() {
                "builtins.object" => Ok(ClassKey::Object),
                "unittest.TestCase" | "unittest.case.TestCase" => Ok(ClassKey::TestCase),
                "unittest.IsolatedAsyncioTestCase"
                | "unittest.async_case.IsolatedAsyncioTestCase" => Ok(ClassKey::AsyncTestCase),
                _ => Err(format!(
                    "external base '{name}' cannot be inspected without importing it"
                )),
            },
            Symbol::Unknown | Symbol::UncertainClass { .. } => {
                let name = match expr {
                    ast::Expr::Name(name) => name.id.as_str(),
                    _ => "<dynamic expression>",
                };
                Err(format!("base '{name}' cannot be resolved statically"))
            }
        }
    }

    fn base_mro(&self, base: ClassKey) -> std::result::Result<Vec<ClassKey>, String> {
        match base {
            ClassKey::Local(index) => self.classes[index].mro.clone(),
            ClassKey::Object => Ok(vec![ClassKey::Object]),
            ClassKey::TestCase => Ok(vec![ClassKey::TestCase, ClassKey::Object]),
            ClassKey::AsyncTestCase => Ok(vec![
                ClassKey::AsyncTestCase,
                ClassKey::TestCase,
                ClassKey::Object,
            ]),
        }
    }

    fn linearize(
        &self,
        index: usize,
        bases: &[std::result::Result<ClassKey, String>],
    ) -> std::result::Result<Vec<ClassKey>, String> {
        let bases = if bases.is_empty() {
            vec![ClassKey::Object]
        } else {
            bases
                .iter()
                .cloned()
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut sequences = bases
            .iter()
            .map(|&base| self.base_mro(base))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        sequences.push(bases);
        let mut positions = vec![0; sequences.len()];
        let mut result = vec![ClassKey::Local(index)];
        while sequences
            .iter()
            .zip(&positions)
            .any(|(sequence, &position)| position < sequence.len())
        {
            let candidate = sequences
                .iter()
                .zip(&positions)
                .filter_map(|(sequence, &position)| sequence.get(position).copied())
                .find(|candidate| {
                    !sequences
                        .iter()
                        .zip(&positions)
                        .any(|(sequence, &position)| {
                            sequence
                                .get(position + 1..)
                                .is_some_and(|tail| tail.contains(candidate))
                        })
                })
                .ok_or_else(|| {
                    "inconsistent or duplicate bases cannot form a Python method resolution order"
                        .to_owned()
                })?;
            result.push(candidate);
            for (sequence, position) in sequences.iter().zip(&mut positions) {
                if sequence.get(*position) == Some(&candidate) {
                    *position += 1;
                }
            }
        }
        Ok(result)
    }

    pub(super) fn collect(
        &self,
        node: &ast::StmtClassDef,
        path: &Path,
        lines: &LineIndex,
    ) -> Result<Vec<TestItem>> {
        let Some(Symbol::Class(ClassKey::Local(index))) = self.symbols.get(node.name.as_str())
        else {
            return Ok(Vec::new());
        };
        let class = &self.classes[*index];
        // A later module definition can replace an earlier class of the same name.
        if class.node.range != node.range {
            return Ok(Vec::new());
        }
        self.collect_named(node.name.as_str(), class, path, lines)
    }

    pub(super) fn collect_aliases(&self, path: &Path, lines: &LineIndex) -> Result<Vec<TestItem>> {
        let mut aliases: Vec<_> = self
            .symbols
            .iter()
            .filter_map(|(name, symbol)| {
                let Symbol::Class(ClassKey::Local(index)) = symbol else {
                    return None;
                };
                let class = &self.classes[*index];
                (name != class.node.name.as_str() && (name.starts_with("Test") || class.unittest))
                    .then_some((name.as_str(), class))
            })
            .collect();
        aliases.sort_by_key(|(name, _)| *name);
        let mut items = Vec::new();
        for (name, class) in aliases {
            items.extend(self.collect_named(name, class, path, lines)?);
        }
        Ok(items)
    }

    fn collect_named(
        &self,
        name: &str,
        class: &Class<'_>,
        path: &Path,
        lines: &LineIndex,
    ) -> Result<Vec<TestItem>> {
        let candidate = name.starts_with("Test") || class.unittest;
        if !candidate {
            if let Err(error) = &class.mro
                && class.bindings.iter().any(|binding| {
                    is_test_name(binding.name)
                        || binding.name.starts_with("test")
                        || binding.name == "runTest"
                })
            {
                return Err(self.collection_error(class, path, lines, error));
            }
            return Ok(Vec::new());
        }
        let mro = match &class.mro {
            Ok(mro) => mro,
            Err(error) => return Err(self.collection_error(class, path, lines, error)),
        };
        let mut class_markers = Vec::new();
        // Borrow decorator ASTs in MRO order. Parameter expansion applies them
        // after method decorators, including parameters inherited from bases.
        let mut class_decorators = Vec::new();
        for key in mro {
            if let ClassKey::Local(index) = key {
                class_decorators.extend(&self.classes[*index].node.decorator_list);
                let inherited =
                    markers::extract_class_markers(&self.classes[*index].node.decorator_list);
                markers::inherit_markers(&mut class_markers, &inherited);
            }
        }
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut run_test = None;
        let mut invalid_run_test = false;
        for key in mro {
            let ClassKey::Local(index) = key else {
                continue;
            };
            let base = &self.classes[*index];
            for statement in &base.node.body {
                reject_compound_tests(path, statement, lines)?;
                let mut nested_test = None;
                visit_scope(statement, &mut |statement| {
                    if let ast::Stmt::ClassDef(nested) = statement {
                        let unittest =
                            nested
                                .bases
                                .iter()
                                .any(|base| match self.resolve_base(base) {
                                    Ok(ClassKey::TestCase | ClassKey::AsyncTestCase) => true,
                                    Ok(ClassKey::Local(index)) => self.classes[index].unittest,
                                    _ => false,
                                });
                        if nested_contains_tests(nested) || unittest {
                            nested_test.get_or_insert(nested);
                        }
                    }
                });
                if let Some(nested) = nested_test {
                    bail!(
                        "Cannot collect {}:{}: nested test class '{}.{}' is unsupported; define test classes at module scope",
                        path.display(),
                        lines.line(nested.range.start().into()),
                        base.node.name,
                        nested.name
                    );
                }
            }
            for binding in &base.bindings {
                let fallback = class.unittest && binding.name == "runTest";
                if !seen.insert(binding.name)
                    || !(fallback
                        || is_test_name(binding.name)
                        || class.unittest && binding.name.starts_with("test"))
                {
                    continue;
                }
                let Some(definition) = binding.definition else {
                    if fallback {
                        invalid_run_test = true;
                        continue;
                    }
                    if !known_non_callable(binding.statement) {
                        bail!(
                            "Cannot collect {}: dynamic test attribute '{}.{}' is unsupported; define test methods with def or async def",
                            path.display(),
                            name,
                            binding.name
                        );
                    }
                    continue;
                };
                let (offset, decorators) = match definition {
                    ast::Stmt::FunctionDef(function) => (
                        usize::from(function.range.start()),
                        &function.decorator_list,
                    ),
                    ast::Stmt::AsyncFunctionDef(function) => (
                        usize::from(function.range.start()),
                        &function.decorator_list,
                    ),
                    _ => continue,
                };
                let mut markers = markers::extract_markers(decorators);
                markers::inherit_markers(&mut markers, &class_markers);
                let item = TestItem {
                    file: path.to_owned(),
                    function: binding.name.to_owned(),
                    class: Some(name.to_owned()),
                    line: lines.line(offset),
                    markers,
                };
                if fallback {
                    run_test = Some((item, decorators));
                } else {
                    crate::parametrize::expand_into(
                        item,
                        class_decorators.iter().copied().chain(decorators),
                        &mut items,
                    )
                    .with_context(|| {
                        format!(
                            "Cannot parametrize {}:{} ({}::{})",
                            path.display(),
                            lines.line(offset),
                            name,
                            binding.name,
                        )
                    })?;
                }
            }
        }
        if items.is_empty() {
            if invalid_run_test {
                bail!(
                    "Cannot collect {}: dynamic runTest attribute on '{}' is unsupported",
                    path.display(),
                    name
                );
            }
            if let Some((run_test, decorators)) = run_test {
                let line = run_test.line;
                crate::parametrize::expand_into(
                    run_test,
                    class_decorators.iter().copied().chain(decorators),
                    &mut items,
                )
                .with_context(|| {
                    format!(
                        "Cannot parametrize {}:{line} ({name}::runTest)",
                        path.display(),
                    )
                })?;
            }
        }
        Ok(items)
    }

    fn collection_error(
        &self,
        class: &Class<'_>,
        path: &Path,
        lines: &LineIndex,
        reason: &str,
    ) -> anyhow::Error {
        anyhow::anyhow!(
            "Cannot collect {}:{}: unsupported inheritance for '{}': {reason}; keep test bases in this module or use unittest.TestCase / unittest.IsolatedAsyncioTestCase",
            path.display(),
            lines.line(class.node.range.start().into()),
            class.node.name
        )
    }
}

fn nested_contains_tests(class: &ast::StmtClassDef) -> bool {
    class.body.iter().any(|stmt| match stmt {
        ast::Stmt::FunctionDef(function) => {
            is_test_name(function.name.as_str())
                || function.name.as_str().starts_with("test")
                || function.name.as_str() == "runTest"
        }
        ast::Stmt::AsyncFunctionDef(function) => {
            is_test_name(function.name.as_str())
                || function.name.as_str().starts_with("test")
                || function.name.as_str() == "runTest"
        }
        ast::Stmt::ClassDef(nested) => nested_contains_tests(nested),
        _ => super::compound_test_offset(stmt).is_some(),
    }) || class.name.as_str().starts_with("Test") && !class.bases.is_empty()
}

pub(super) fn target_names(expr: &ast::Expr) -> Vec<&str> {
    match expr {
        ast::Expr::Name(name) => vec![name.id.as_str()],
        ast::Expr::Tuple(tuple) => tuple.elts.iter().flat_map(target_names).collect(),
        ast::Expr::List(list) => list.elts.iter().flat_map(target_names).collect(),
        ast::Expr::Starred(starred) => target_names(&starred.value),
        _ => Vec::new(),
    }
}

fn class_bindings(body: &[ast::Stmt]) -> Vec<Binding<'_>> {
    let mut indices = HashMap::new();
    let mut bindings: Vec<Option<Binding<'_>>> = Vec::new();
    for stmt in body {
        if let ast::Stmt::Delete(delete) = stmt {
            for name in delete.targets.iter().flat_map(target_names) {
                if let Some(index) = indices.remove(name) {
                    bindings[index] = None;
                }
            }
            continue;
        }
        let names = statement_names(stmt);
        let definition = matches!(
            stmt,
            ast::Stmt::FunctionDef(_) | ast::Stmt::AsyncFunctionDef(_)
        )
        .then_some(stmt);
        for name in names {
            let binding = Some(Binding {
                name,
                definition,
                statement: stmt,
            });
            if let Some(&index) = indices.get(name) {
                bindings[index] = binding;
            } else {
                indices.insert(name, bindings.len());
                bindings.push(binding);
            }
        }
    }
    bindings.into_iter().flatten().collect()
}

// Literal values reliably shadow inherited callable methods. Arbitrary
// expressions and imported aliases may create tests, so cannot be ignored.
fn known_non_callable(statement: &ast::Stmt) -> bool {
    let value = match statement {
        ast::Stmt::Assign(assign)
            if assign
                .targets
                .iter()
                .all(|target| matches!(target, ast::Expr::Name(_))) =>
        {
            Some(assign.value.as_ref())
        }
        ast::Stmt::AnnAssign(assign) => assign.value.as_deref(),
        ast::Stmt::ClassDef(_) => return true,
        _ => None,
    };
    value.is_some_and(|value| {
        matches!(
            value,
            ast::Expr::Constant(_)
                | ast::Expr::List(_)
                | ast::Expr::Tuple(_)
                | ast::Expr::Set(_)
                | ast::Expr::Dict(_)
        )
    })
}

// Return names bound in this scope. Function and class bodies create new scopes;
// compound statement bodies do not. Used to invalidate uncertain base aliases
// and to reject conditional creation or shadowing of test attributes.
fn statement_names(stmt: &ast::Stmt) -> Vec<&str> {
    fn suite(body: &[ast::Stmt]) -> Vec<&str> {
        body.iter().flat_map(statement_names).collect()
    }
    match stmt {
        ast::Stmt::FunctionDef(node) => vec![node.name.as_str()],
        ast::Stmt::AsyncFunctionDef(node) => vec![node.name.as_str()],
        ast::Stmt::ClassDef(node) => vec![node.name.as_str()],
        ast::Stmt::Assign(node) => node.targets.iter().flat_map(target_names).collect(),
        ast::Stmt::AnnAssign(node) if node.value.is_some() => target_names(&node.target),
        ast::Stmt::AugAssign(node) => target_names(&node.target),
        ast::Stmt::Delete(node) => node.targets.iter().flat_map(target_names).collect(),
        ast::Stmt::Import(node) => node
            .names
            .iter()
            .map(|alias| {
                alias.asname.as_ref().map_or_else(
                    || {
                        alias
                            .name
                            .as_str()
                            .split('.')
                            .next()
                            .unwrap_or(alias.name.as_str())
                    },
                    |name| name.as_str(),
                )
            })
            .collect(),
        ast::Stmt::ImportFrom(node) => node
            .names
            .iter()
            .map(|alias| alias.asname.as_ref().unwrap_or(&alias.name).as_str())
            .collect(),
        ast::Stmt::If(node) => suite(&node.body)
            .into_iter()
            .chain(suite(&node.orelse))
            .collect(),
        ast::Stmt::While(node) => suite(&node.body)
            .into_iter()
            .chain(suite(&node.orelse))
            .collect(),
        ast::Stmt::For(node) => target_names(&node.target)
            .into_iter()
            .chain(suite(&node.body))
            .chain(suite(&node.orelse))
            .collect(),
        ast::Stmt::AsyncFor(node) => target_names(&node.target)
            .into_iter()
            .chain(suite(&node.body))
            .chain(suite(&node.orelse))
            .collect(),
        ast::Stmt::With(node) => node
            .items
            .iter()
            .filter_map(|item| item.optional_vars.as_deref())
            .flat_map(target_names)
            .chain(suite(&node.body))
            .collect(),
        ast::Stmt::AsyncWith(node) => node
            .items
            .iter()
            .filter_map(|item| item.optional_vars.as_deref())
            .flat_map(target_names)
            .chain(suite(&node.body))
            .collect(),
        ast::Stmt::Match(node) => node
            .cases
            .iter()
            .flat_map(|case| suite(&case.body))
            .collect(),
        ast::Stmt::Try(node) => suite(&node.body)
            .into_iter()
            .chain(suite(&node.orelse))
            .chain(suite(&node.finalbody))
            .chain(node.handlers.iter().flat_map(|handler| {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                handler
                    .name
                    .iter()
                    .map(|name| name.as_str())
                    .chain(suite(&handler.body))
            }))
            .collect(),
        ast::Stmt::TryStar(node) => suite(&node.body)
            .into_iter()
            .chain(suite(&node.orelse))
            .chain(suite(&node.finalbody))
            .chain(node.handlers.iter().flat_map(|handler| {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                handler
                    .name
                    .iter()
                    .map(|name| name.as_str())
                    .chain(suite(&handler.body))
            }))
            .collect(),
        _ => Vec::new(),
    }
}

// Visit statements sharing the current scope, including control-flow branches,
// without descending into function or class scopes.
pub(super) fn visit_scope<'a>(stmt: &'a ast::Stmt, visitor: &mut impl FnMut(&'a ast::Stmt)) {
    visitor(stmt);
    let mut visit_body = |body: &'a [ast::Stmt]| {
        for statement in body {
            visit_scope(statement, visitor);
        }
    };
    match stmt {
        ast::Stmt::If(node) => {
            visit_body(&node.body);
            visit_body(&node.orelse);
        }
        ast::Stmt::While(node) => {
            visit_body(&node.body);
            visit_body(&node.orelse);
        }
        ast::Stmt::For(node) => {
            visit_body(&node.body);
            visit_body(&node.orelse);
        }
        ast::Stmt::AsyncFor(node) => {
            visit_body(&node.body);
            visit_body(&node.orelse);
        }
        ast::Stmt::With(node) => visit_body(&node.body),
        ast::Stmt::AsyncWith(node) => visit_body(&node.body),
        ast::Stmt::Match(node) => {
            for case in &node.cases {
                visit_body(&case.body);
            }
        }
        ast::Stmt::Try(node) => {
            visit_body(&node.body);
            visit_body(&node.orelse);
            visit_body(&node.finalbody);
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                visit_body(&handler.body);
            }
        }
        ast::Stmt::TryStar(node) => {
            visit_body(&node.body);
            visit_body(&node.orelse);
            visit_body(&node.finalbody);
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                visit_body(&handler.body);
            }
        }
        _ => {}
    }
}
