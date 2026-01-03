use anyhow::{Context, Result};
use rustpython_parser::Parse;
use rustpython_parser::ast::{self, Ranged};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use xxhash_rust::xxh64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Function,
    Method,
    Class,
    TopLevel,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct BlockId {
    pub file: PathBuf,
    pub kind: BlockKind,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub checksum: String,
}

#[derive(Debug, Default)]
pub struct FileBlocks {
    pub file: PathBuf,
    pub blocks: Vec<Block>,
    pub line_to_block: HashMap<usize, usize>, // line_number -> block index
}

impl FileBlocks {
    pub fn from_file(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let ast = ast::Suite::parse(&source, "<module>")
            .map_err(|e| anyhow::anyhow!("Parse error in {}: {}", path.display(), e))?;

        let mut blocks = Vec::new();

        // Extract imports as a single block
        Self::extract_imports(&ast, &source, path, &mut blocks);

        // Extract top-level code
        Self::extract_top_level(&ast, &source, path, &mut blocks);

        // Extract functions and classes
        Self::extract_definitions(&ast, &source, path, &mut blocks, None);

        // Build line -> block index mapping
        let mut line_to_block = HashMap::new();
        for (idx, block) in blocks.iter().enumerate() {
            for line in block.id.start_line..=block.id.end_line {
                line_to_block.insert(line, idx);
            }
        }

        Ok(Self {
            file: path.to_path_buf(),
            blocks,
            line_to_block,
        })
    }

    pub fn get_block_for_line(&self, line: usize) -> Option<&Block> {
        self.line_to_block.get(&line).map(|&idx| &self.blocks[idx])
    }

    fn extract_imports(ast: &[ast::Stmt], source: &str, file: &Path, blocks: &mut Vec<Block>) {
        let mut import_lines: Vec<(usize, usize)> = Vec::new();

        for stmt in ast {
            match stmt {
                ast::Stmt::Import(imp) => {
                    let start = offset_to_line(source, imp.range.start().into());
                    let end = offset_to_line(source, imp.range.end().into());
                    import_lines.push((start, end));
                }
                ast::Stmt::ImportFrom(imp) => {
                    let start = offset_to_line(source, imp.range.start().into());
                    let end = offset_to_line(source, imp.range.end().into());
                    import_lines.push((start, end));
                }
                _ => {}
            }
        }

        if import_lines.is_empty() {
            return;
        }

        let min_line = import_lines.iter().map(|(s, _)| *s).min().unwrap();
        let max_line = import_lines.iter().map(|(_, e)| *e).max().unwrap();
        let source_slice = extract_lines(source, min_line, max_line);

        blocks.push(Block {
            id: BlockId {
                file: file.to_path_buf(),
                kind: BlockKind::Import,
                name: "<imports>".to_string(),
                start_line: min_line,
                end_line: max_line,
            },
            checksum: compute_checksum(&source_slice),
        });
    }

    fn extract_top_level(ast: &[ast::Stmt], source: &str, file: &Path, blocks: &mut Vec<Block>) {
        let mut top_level_ranges: Vec<(usize, usize)> = Vec::new();

        for stmt in ast {
            match stmt {
                ast::Stmt::Import(_)
                | ast::Stmt::ImportFrom(_)
                | ast::Stmt::FunctionDef(_)
                | ast::Stmt::ClassDef(_) => continue,
                _ => {
                    let start = offset_to_line(source, stmt.range().start().into());
                    let end = offset_to_line(source, stmt.range().end().into());
                    top_level_ranges.push((start, end));
                }
            }
        }

        if top_level_ranges.is_empty() {
            return;
        }

        // Merge consecutive statements
        let mut current_start = top_level_ranges[0].0;
        let mut current_end = top_level_ranges[0].1;
        let mut block_num = 0;

        let add_block = |start: usize, end: usize, num: usize| -> Block {
            let source_slice = extract_lines(source, start, end);
            Block {
                id: BlockId {
                    file: file.to_path_buf(),
                    kind: BlockKind::TopLevel,
                    name: format!("<toplevel_{}>", num),
                    start_line: start,
                    end_line: end,
                },
                checksum: compute_checksum(&source_slice),
            }
        };

        for i in 1..top_level_ranges.len() {
            let (start, end) = top_level_ranges[i];
            if start <= current_end + 2 {
                current_end = end;
            } else {
                blocks.push(add_block(current_start, current_end, block_num));
                block_num += 1;
                current_start = start;
                current_end = end;
            }
        }

        blocks.push(add_block(current_start, current_end, block_num));
    }

    fn extract_definitions(
        ast: &[ast::Stmt],
        source: &str,
        file: &Path,
        blocks: &mut Vec<Block>,
        parent_class: Option<&str>,
    ) {
        for stmt in ast {
            match stmt {
                ast::Stmt::FunctionDef(func) => {
                    // Start from decorator if present, otherwise from def line
                    let start = if !func.decorator_list.is_empty() {
                        offset_to_line(source, func.decorator_list[0].range().start().into())
                    } else {
                        offset_to_line(source, func.range.start().into())
                    };
                    let end = offset_to_line(source, func.range.end().into());
                    let source_slice = extract_lines(source, start, end);

                    let (kind, name) = if let Some(cls) = parent_class {
                        (BlockKind::Method, format!("{}.{}", cls, func.name))
                    } else {
                        (BlockKind::Function, func.name.to_string())
                    };

                    blocks.push(Block {
                        id: BlockId {
                            file: file.to_path_buf(),
                            kind,
                            name,
                            start_line: start,
                            end_line: end,
                        },
                        checksum: compute_checksum(&source_slice),
                    });
                }
                ast::Stmt::AsyncFunctionDef(func) => {
                    // Same logic as FunctionDef - async functions have the same structure
                    let start = if !func.decorator_list.is_empty() {
                        offset_to_line(source, func.decorator_list[0].range().start().into())
                    } else {
                        offset_to_line(source, func.range.start().into())
                    };
                    let end = offset_to_line(source, func.range.end().into());
                    let source_slice = extract_lines(source, start, end);

                    let (kind, name) = if let Some(cls) = parent_class {
                        (BlockKind::Method, format!("{}.{}", cls, func.name))
                    } else {
                        (BlockKind::Function, func.name.to_string())
                    };

                    blocks.push(Block {
                        id: BlockId {
                            file: file.to_path_buf(),
                            kind,
                            name,
                            start_line: start,
                            end_line: end,
                        },
                        checksum: compute_checksum(&source_slice),
                    });
                }
                ast::Stmt::ClassDef(class) => {
                    let start = offset_to_line(source, class.range.start().into());
                    let end = offset_to_line(source, class.range.end().into());

                    // Class header (before first method)
                    let header_end = class
                        .body
                        .iter()
                        .filter_map(|s| {
                            if matches!(
                                s,
                                ast::Stmt::FunctionDef(_) | ast::Stmt::AsyncFunctionDef(_)
                            ) {
                                Some(offset_to_line(source, s.range().start().into()) - 1)
                            } else {
                                None
                            }
                        })
                        .min()
                        .unwrap_or(end);

                    let class_source = extract_lines(source, start, header_end);
                    blocks.push(Block {
                        id: BlockId {
                            file: file.to_path_buf(),
                            kind: BlockKind::Class,
                            name: class.name.to_string(),
                            start_line: start,
                            end_line: header_end,
                        },
                        checksum: compute_checksum(&class_source),
                    });

                    // Recursively extract methods
                    Self::extract_definitions(&class.body, source, file, blocks, Some(&class.name));
                }
                _ => {}
            }
        }
    }
}

fn compute_checksum(source: &str) -> String {
    let normalized: String = source
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    let hash = xxh64::xxh64(normalized.as_bytes(), 0);
    format!("{:x}", hash)
}

fn extract_lines(source: &str, start: usize, end: usize) -> String {
    source
        .lines()
        .enumerate()
        .filter(|(i, _)| *i + 1 >= start && *i < end)
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn offset_to_line(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_ignores_whitespace() {
        let a = compute_checksum("def foo():\n    pass");
        let b = compute_checksum("def foo():\n        pass");
        assert_eq!(a, b);
    }

    #[test]
    fn test_checksum_ignores_comments() {
        let a = compute_checksum("def foo():\n    pass");
        let b = compute_checksum("def foo():\n    # comment\n    pass");
        assert_eq!(a, b);
    }

    #[test]
    fn test_checksum_detects_changes() {
        let a = compute_checksum("def foo():\n    return 1");
        let b = compute_checksum("def foo():\n    return 2");
        assert_ne!(a, b);
    }
}
