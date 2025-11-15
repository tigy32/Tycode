use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use cargo_metadata::MetadataCommand;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use syn::{Item, ItemImpl, ItemTrait};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::{BuildStatus, TypeAnalyzer};

pub struct RustAnalyzer {
    workspace_root: PathBuf,
}

struct ItemWithImpls {
    item: Item,
    impls: Vec<ItemImpl>,
    traits: Vec<ItemTrait>,
}

impl RustAnalyzer {
    pub fn new(workspace_root: PathBuf) -> Self {
        RustAnalyzer { workspace_root }
    }
}

#[async_trait]
impl TypeAnalyzer for RustAnalyzer {
    async fn search_types_by_name(&mut self, type_name: &str) -> Result<Vec<String>> {
        let metadata = MetadataCommand::new()
            .current_dir(&self.workspace_root)
            .exec()
            .context("failed to run cargo metadata")?;

        let mut results = Vec::new();
        let limit = 20;

        // Partition packages: workspace members first, then dependencies
        let (workspace_packages, dependency_packages): (Vec<_>, Vec<_>) =
            metadata.packages.iter().partition(|pkg| {
                pkg.manifest_path
                    .as_std_path()
                    .starts_with(&self.workspace_root)
            });

        // Search workspace packages first
        for package in &workspace_packages {
            let crate_name = package.name.replace('-', "_");
            let crate_root = package
                .manifest_path
                .parent()
                .context("manifest has no parent directory")?;

            let found = search_crate_for_type(&crate_root.into(), &crate_name, type_name, limit);
            results.extend(found);

            if results.len() >= limit {
                results.truncate(limit);
                break;
            }
        }

        // Search dependencies if nothing found in workspace
        if results.is_empty() {
            for package in &dependency_packages {
                let crate_name = package.name.replace('-', "_");
                let crate_root = package
                    .manifest_path
                    .parent()
                    .context("manifest has no parent directory")?;

                let found =
                    search_crate_for_type(&crate_root.into(), &crate_name, type_name, limit);
                results.extend(found);

                if results.len() >= limit {
                    results.truncate(limit);
                    break;
                }
            }
        }

        if results.is_empty() {
            bail!("no types found matching '{}'", type_name);
        }

        Ok(results)
    }

    async fn get_type_docs(&mut self, type_path: &str) -> Result<String> {
        let parts: Vec<&str> = type_path.split("::").collect();
        if parts.is_empty() {
            bail!("empty type path");
        }

        let crate_name = parts[0];
        let item_path = &parts[1..];

        let source_path = find_crate_source(crate_name, &self.workspace_root)?;
        let item_with_impls = find_item_in_source(&source_path, item_path)?;
        let code_outline = format_item_as_code(&item_with_impls);

        Ok(code_outline)
    }

    async fn get_build_status(&mut self) -> Result<BuildStatus> {
        let mut child = Command::new("cargo")
            .args(["check", "--tests", "--message-format=json"])
            .current_dir(&self.workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn cargo check")?;

        let stdout = child.stdout.take().context("failed to capture stdout")?;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        while let Some(line) = lines.next_line().await? {
            let Ok(message) = serde_json::from_str::<CargoMessage>(&line) else {
                continue;
            };

            let Some(compiler_message) = message.message else {
                continue;
            };

            let formatted = format_compiler_message(&compiler_message);

            match compiler_message.level.as_str() {
                "error" => errors.push(formatted),
                "warning" => warnings.push(formatted),
                _ => {}
            }
        }

        child.wait().await?;

        Ok(BuildStatus { errors, warnings })
    }
}

#[derive(Deserialize)]
struct CargoMessage {
    message: Option<CompilerMessage>,
}

#[derive(Deserialize)]
struct CompilerMessage {
    message: String,
    level: String,
    spans: Vec<CompilerSpan>,
}

#[derive(Deserialize)]
struct CompilerSpan {
    file_name: String,
    line_start: u32,
    column_start: u32,
}

fn format_compiler_message(msg: &CompilerMessage) -> String {
    if msg.spans.is_empty() {
        return msg.message.clone();
    }

    let span = &msg.spans[0];
    format!(
        "{}:{}:{}: {}",
        span.file_name, span.line_start, span.column_start, msg.message
    )
}

fn search_crate_for_type(
    crate_root: &PathBuf,
    crate_name: &str,
    type_name: &str,
    limit: usize,
) -> Vec<String> {
    // Run parsing in a thread with larger stack to handle complex syntax trees
    let crate_root = crate_root.clone();
    let crate_name = crate_name.to_string();
    let type_name = type_name.to_string();

    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024) // 8MB stack
        .spawn(move || search_crate_for_type_inner(&crate_root, &crate_name, &type_name, limit))
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

fn search_crate_for_type_inner(
    crate_root: &PathBuf,
    crate_name: &str,
    type_name: &str,
    limit: usize,
) -> Vec<String> {
    let lib_path = crate_root.join("src").join("lib.rs");
    if !lib_path.exists() {
        return Vec::new();
    }

    let Ok(content) = std::fs::read_to_string(&lib_path) else {
        return Vec::new();
    };

    let file_size = content.len();

    if file_size > 500_000 {
        return Vec::new();
    }

    let Ok(file) = syn::parse_file(&content) else {
        return Vec::new();
    };

    let mut results = Vec::new();
    let src_dir = crate_root.join("src");
    collect_matching_types_iterative(
        &file.items,
        type_name,
        crate_name,
        &src_dir,
        &mut results,
        limit,
    );

    results
}

/// Work item for iterative type search
struct TypeSearchWork {
    items: Vec<Item>,
    path: String,
    dir: PathBuf,
}

fn collect_matching_types_iterative(
    initial_items: &[Item],
    type_name: &str,
    initial_path: &str,
    initial_dir: &Path,
    results: &mut Vec<String>,
    limit: usize,
) {
    // Use explicit stack on heap instead of call stack
    let mut work_stack: Vec<TypeSearchWork> = vec![TypeSearchWork {
        items: initial_items.to_vec(),
        path: initial_path.to_string(),
        dir: initial_dir.to_path_buf(),
    }];

    while let Some(work) = work_stack.pop() {
        if results.len() >= limit {
            break;
        }

        for item in &work.items {
            if results.len() >= limit {
                break;
            }

            match item {
                Item::Struct(s) if s.ident == type_name => {
                    results.push(format!("{}::{}", work.path, s.ident));
                }
                Item::Enum(e) if e.ident == type_name => {
                    results.push(format!("{}::{}", work.path, e.ident));
                }
                Item::Trait(t) if t.ident == type_name => {
                    results.push(format!("{}::{}", work.path, t.ident));
                }
                Item::Type(t) if t.ident == type_name => {
                    results.push(format!("{}::{}", work.path, t.ident));
                }
                Item::Union(u) if u.ident == type_name => {
                    results.push(format!("{}::{}", work.path, u.ident));
                }
                Item::Fn(f) if f.sig.ident == type_name => {
                    results.push(format!("{}::{}", work.path, f.sig.ident));
                }
                Item::Mod(m) => {
                    let nested_path = format!("{}::{}", work.path, m.ident);

                    if let Some((_, nested_items)) = &m.content {
                        work_stack.push(TypeSearchWork {
                            items: nested_items.clone(),
                            path: nested_path,
                            dir: work.dir.clone(),
                        });
                        continue;
                    }

                    let module_name = m.ident.to_string();
                    let nested_items = match load_module_items(&module_name, &work.dir) {
                        Ok(items) => items,
                        Err(_e) => {
                            // Silent failure for module loading - common for large/missing modules
                            continue;
                        }
                    };
                    let module_dir = resolve_module_dir(&module_name, &work.dir);
                    work_stack.push(TypeSearchWork {
                        items: nested_items,
                        path: nested_path,
                        dir: module_dir,
                    });
                }
                _ => {}
            }
        }
    }
}

fn find_crate_source(crate_name: &str, workspace_root: &PathBuf) -> Result<PathBuf> {
    let metadata = MetadataCommand::new()
        .current_dir(workspace_root)
        .exec()
        .context("failed to run cargo metadata")?;

    for package in &metadata.packages {
        if crate_name == package.name.replace('-', "_") {
            let parent = package
                .manifest_path
                .parent()
                .context("manifest has no parent directory")?;
            return Ok(parent.into());
        }
    }

    bail!("crate '{}' not found in dependencies", crate_name)
}

fn find_item_in_source(crate_root: &PathBuf, item_path: &[&str]) -> Result<ItemWithImpls> {
    let lib_path = crate_root.join("src").join("lib.rs");
    if !lib_path.exists() {
        bail!("lib.rs not found at {:?}", lib_path);
    }

    let content = std::fs::read_to_string(&lib_path).context("failed to read source file")?;

    let file = syn::parse_file(&content).context("failed to parse source file")?;

    if item_path.is_empty() {
        bail!("no item specified");
    }

    let src_dir = crate_root.join("src");
    let item = search_items_iterative(&file.items, item_path, &src_dir)?;
    let item_name = extract_item_name(&item)?;
    let impls = collect_impls_for_item_iterative(&file.items, &item_name, &src_dir);

    let mut traits = Vec::new();
    let mut seen_traits = HashSet::new();

    for impl_item in &impls {
        if let Some(trait_path) = extract_trait_path_from_impl(impl_item) {
            if !is_std_trait(&trait_path) {
                let trait_name = trait_path.split("::").last().unwrap_or(&trait_path);
                if seen_traits.insert(trait_name.to_string()) {
                    if let Some(trait_def) =
                        find_trait_in_items_iterative(&file.items, trait_name, &src_dir)
                    {
                        traits.push(trait_def);
                    }
                }
            }
        }
    }

    Ok(ItemWithImpls {
        item,
        impls,
        traits,
    })
}

/// Work item for iterative item search
struct ItemSearchWork {
    items: Vec<Item>,
    depth: usize,
    dir: PathBuf,
}

fn search_items_iterative(
    initial_items: &[Item],
    path: &[&str],
    initial_dir: &Path,
) -> Result<Item> {
    if path.is_empty() {
        bail!("empty path");
    }

    let mut work_stack: Vec<ItemSearchWork> = vec![ItemSearchWork {
        items: initial_items.to_vec(),
        depth: 0,
        dir: initial_dir.to_path_buf(),
    }];

    while let Some(work) = work_stack.pop() {
        if work.depth >= path.len() {
            continue;
        }

        let target_name = path[work.depth];
        let is_final = work.depth == path.len() - 1;

        for item in &work.items {
            let matches = match item {
                Item::Struct(s) => s.ident == target_name && is_final,
                Item::Enum(e) => e.ident == target_name && is_final,
                Item::Trait(t) => t.ident == target_name && is_final,
                Item::Type(t) => t.ident == target_name && is_final,
                Item::Union(u) => u.ident == target_name && is_final,
                Item::Fn(f) => f.sig.ident == target_name && is_final,
                Item::Mod(m) if m.ident == target_name && !is_final => {
                    if let Some((_, nested_items)) = &m.content {
                        work_stack.push(ItemSearchWork {
                            items: nested_items.clone(),
                            depth: work.depth + 1,
                            dir: work.dir.clone(),
                        });
                        continue;
                    }
                    let Ok(nested_items) = load_module_items(&m.ident.to_string(), &work.dir)
                    else {
                        continue;
                    };
                    let module_dir = resolve_module_dir(&m.ident.to_string(), &work.dir);
                    work_stack.push(ItemSearchWork {
                        items: nested_items,
                        depth: work.depth + 1,
                        dir: module_dir,
                    });
                    continue;
                }
                _ => false,
            };

            if matches {
                return Ok(item.clone());
            }
        }

        // If we're at the final segment and didn't find it, search all modules
        if is_final {
            if let Some(found) = search_all_modules_iterative(&work.items, target_name, &work.dir) {
                return Ok(found);
            }
        }
    }

    bail!("item '{}' not found", path.last().unwrap_or(&""))
}

fn extract_item_name(item: &Item) -> Result<String> {
    let name = match item {
        Item::Struct(s) => s.ident.to_string(),
        Item::Enum(e) => e.ident.to_string(),
        Item::Trait(t) => t.ident.to_string(),
        Item::Type(t) => t.ident.to_string(),
        Item::Union(u) => u.ident.to_string(),
        Item::Fn(f) => f.sig.ident.to_string(),
        _ => bail!("unsupported item type for name extraction"),
    };
    Ok(name)
}

/// Work item for iterative module search
struct ModuleSearchWork {
    items: Vec<Item>,
    dir: PathBuf,
}

fn search_all_modules_iterative(
    initial_items: &[Item],
    target_name: &str,
    initial_dir: &Path,
) -> Option<Item> {
    let mut work_stack: Vec<ModuleSearchWork> = vec![ModuleSearchWork {
        items: initial_items.to_vec(),
        dir: initial_dir.to_path_buf(),
    }];

    while let Some(work) = work_stack.pop() {
        for item in &work.items {
            match item {
                Item::Struct(s) if s.ident == target_name => return Some(item.clone()),
                Item::Enum(e) if e.ident == target_name => return Some(item.clone()),
                Item::Trait(t) if t.ident == target_name => return Some(item.clone()),
                Item::Type(t) if t.ident == target_name => return Some(item.clone()),
                Item::Union(u) if u.ident == target_name => return Some(item.clone()),
                Item::Mod(m) => {
                    if let Some((_, nested_items)) = &m.content {
                        work_stack.push(ModuleSearchWork {
                            items: nested_items.clone(),
                            dir: work.dir.clone(),
                        });
                        continue;
                    }

                    let Ok(nested_items) = load_module_items(&m.ident.to_string(), &work.dir)
                    else {
                        continue;
                    };
                    let module_dir = resolve_module_dir(&m.ident.to_string(), &work.dir);
                    work_stack.push(ModuleSearchWork {
                        items: nested_items,
                        dir: module_dir,
                    });
                }
                _ => {}
            }
        }
    }
    None
}

/// Work item for iterative impl collection
struct ImplSearchWork {
    items: Vec<Item>,
    dir: PathBuf,
}

fn collect_impls_for_item_iterative(
    initial_items: &[Item],
    target_name: &str,
    initial_dir: &Path,
) -> Vec<ItemImpl> {
    let mut impls = Vec::new();
    let mut work_stack: Vec<ImplSearchWork> = vec![ImplSearchWork {
        items: initial_items.to_vec(),
        dir: initial_dir.to_path_buf(),
    }];

    while let Some(work) = work_stack.pop() {
        for item in &work.items {
            if let Item::Impl(impl_item) = item {
                if impl_matches_type(impl_item, target_name) {
                    impls.push(impl_item.clone());
                }
                continue;
            }

            let Item::Mod(m) = item else {
                continue;
            };

            if let Some((_, nested_items)) = &m.content {
                work_stack.push(ImplSearchWork {
                    items: nested_items.clone(),
                    dir: work.dir.clone(),
                });
                continue;
            }

            let Ok(nested_items) = load_module_items(&m.ident.to_string(), &work.dir) else {
                continue;
            };
            let module_dir = resolve_module_dir(&m.ident.to_string(), &work.dir);
            work_stack.push(ImplSearchWork {
                items: nested_items,
                dir: module_dir,
            });
        }
    }

    impls
}

fn impl_matches_type(impl_item: &ItemImpl, target_name: &str) -> bool {
    if let syn::Type::Path(type_path) = &*impl_item.self_ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == target_name;
        }
    }
    false
}

fn extract_trait_path_from_impl(impl_item: &ItemImpl) -> Option<String> {
    impl_item.trait_.as_ref().map(|(_, path, _)| {
        path.segments
            .iter()
            .map(|seg| seg.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    })
}

fn is_std_trait(trait_path: &str) -> bool {
    trait_path.starts_with("std::")
        || trait_path.starts_with("core::")
        || trait_path.starts_with("alloc::")
}

fn load_module_items(module_name: &str, current_dir: &Path) -> Result<Vec<Item>> {
    // Strip r# prefix for raw identifiers (e.g., r#trait -> trait)
    let file_name = module_name.strip_prefix("r#").unwrap_or(module_name);

    let mod_file = current_dir.join(format!("{}.rs", file_name));
    let mod_dir_file = current_dir.join(file_name).join("mod.rs");

    let path = if mod_file.exists() {
        mod_file
    } else if mod_dir_file.exists() {
        mod_dir_file
    } else {
        bail!(
            "module file not found for '{}' in {:?}",
            module_name,
            current_dir
        );
    };

    let content =
        std::fs::read_to_string(&path).context(format!("failed to read module file {:?}", path))?;

    let file_size = content.len();

    if file_size > 500_000 {
        bail!("skipping large module (>500KB)");
    }

    let file =
        syn::parse_file(&content).context(format!("failed to parse module file {:?}", path))?;

    Ok(file.items)
}

fn resolve_module_dir(module_name: &str, current_dir: &Path) -> PathBuf {
    // Strip r# prefix for raw identifiers
    let file_name = module_name.strip_prefix("r#").unwrap_or(module_name);

    let mod_file = current_dir.join(format!("{}.rs", file_name));
    if mod_file.exists() {
        return current_dir.join(file_name);
    }
    current_dir.join(file_name)
}

/// Work item for iterative trait search
struct TraitSearchWork {
    items: Vec<Item>,
    dir: PathBuf,
}

fn find_trait_in_items_iterative(
    initial_items: &[Item],
    trait_name: &str,
    initial_dir: &Path,
) -> Option<ItemTrait> {
    let mut work_stack: Vec<TraitSearchWork> = vec![TraitSearchWork {
        items: initial_items.to_vec(),
        dir: initial_dir.to_path_buf(),
    }];

    while let Some(work) = work_stack.pop() {
        for item in &work.items {
            match item {
                Item::Trait(t) if t.ident == trait_name => {
                    return Some(t.clone());
                }
                Item::Mod(m) => {
                    if let Some((_, nested_items)) = &m.content {
                        work_stack.push(TraitSearchWork {
                            items: nested_items.clone(),
                            dir: work.dir.clone(),
                        });
                        continue;
                    }

                    let Ok(nested_items) = load_module_items(&m.ident.to_string(), &work.dir)
                    else {
                        continue;
                    };
                    let module_dir = resolve_module_dir(&m.ident.to_string(), &work.dir);
                    work_stack.push(TraitSearchWork {
                        items: nested_items,
                        dir: module_dir,
                    });
                }
                _ => {}
            }
        }
    }
    None
}

fn format_item_as_code(item_with_impls: &ItemWithImpls) -> String {
    let mut formatted_items = Vec::new();

    for trait_item in &item_with_impls.traits {
        let item = Item::Trait(strip_trait_bodies(trait_item));
        let file = syn::File {
            shebang: None,
            attrs: vec![],
            items: vec![item],
        };
        let formatted = prettyplease::unparse(&file);
        let formatted = replace_empty_bodies_with_semicolons(&formatted);
        formatted_items.push(add_blank_lines_between_items(&formatted));
    }

    let main_item = strip_implementations(&item_with_impls.item);
    let file = syn::File {
        shebang: None,
        attrs: vec![],
        items: vec![main_item],
    };
    formatted_items.push(prettyplease::unparse(&file));

    for impl_item in &item_with_impls.impls {
        let item = Item::Impl(strip_impl_bodies(impl_item));
        let file = syn::File {
            shebang: None,
            attrs: vec![],
            items: vec![item],
        };
        let formatted = prettyplease::unparse(&file);
        let formatted = replace_empty_bodies_with_semicolons(&formatted);
        formatted_items.push(add_blank_lines_between_items(&formatted));
    }

    let combined = formatted_items.join("\n\n");
    format!("```rust\n{}```", combined)
}

fn strip_implementations(item: &Item) -> Item {
    match item {
        Item::Fn(f) => {
            let mut func = f.clone();
            func.block = Box::new(syn::Block {
                brace_token: syn::token::Brace::default(),
                stmts: vec![],
            });
            Item::Fn(func)
        }
        _ => item.clone(),
    }
}

fn add_blank_lines_between_items(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        result.push(lines[i].to_string());

        if i + 1 < lines.len() {
            let current_line = lines[i].trim();
            let next_line = lines[i + 1].trim();

            let current_is_doc_comment = current_line.starts_with("///");
            let next_is_doc_comment = next_line.starts_with("///");

            if current_is_doc_comment && next_is_doc_comment {
                i += 1;
                continue;
            }

            let current_ends_item = current_line.ends_with('}') || current_line.ends_with(';');
            let next_starts_new_item = next_line.starts_with("///")
                || next_line.starts_with("#[")
                || next_line.starts_with("pub fn")
                || next_line.starts_with("fn")
                || next_line.starts_with("async fn")
                || next_line.starts_with("pub async fn")
                || next_line.starts_with("pub(crate) fn")
                || next_line.starts_with("pub(super) fn")
                || next_line.starts_with("unsafe fn")
                || next_line.starts_with("pub unsafe fn");

            if current_ends_item && next_starts_new_item && !next_line.is_empty() {
                result.push(String::new());
            }
        }

        i += 1;
    }

    result.join("\n")
}

fn replace_empty_bodies_with_semicolons(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut result: Vec<String> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed == "{}" {
            if let Some(last_line) = result.last_mut() {
                let last_trimmed = last_line.trim_end();
                if last_trimmed.ends_with(',') {
                    let without_comma = &last_trimmed[..last_trimmed.len() - 1];
                    *last_line = format!(
                        "{}{}",
                        &last_line[..last_line.len() - last_trimmed.len()],
                        without_comma.to_string() + ";"
                    );
                } else {
                    *last_line = last_line.trim_end().to_string() + ";";
                }
            }
            i += 1;
            continue;
        }

        if trimmed.ends_with(" {}") {
            let without_braces = &trimmed[..trimmed.len() - 3];
            result.push(format!(
                "{}{}",
                &line[..line.len() - trimmed.len()],
                without_braces.to_string() + ";"
            ));
        } else {
            result.push(line.to_string());
        }

        i += 1;
    }

    result.join("\n")
}

fn strip_impl_bodies(impl_item: &ItemImpl) -> ItemImpl {
    let mut impl_clone = impl_item.clone();

    for item in &mut impl_clone.items {
        if let syn::ImplItem::Fn(method) = item {
            method.block = syn::Block {
                brace_token: syn::token::Brace::default(),
                stmts: vec![],
            };
        }
    }

    impl_clone
}

fn strip_trait_bodies(trait_item: &ItemTrait) -> ItemTrait {
    let mut trait_clone = trait_item.clone();

    for item in &mut trait_clone.items {
        if let syn::TraitItem::Fn(method) = item {
            method.default = None;
        }
    }

    trait_clone
}
