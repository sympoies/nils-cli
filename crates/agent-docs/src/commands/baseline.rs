use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::load_configs_from_roots;
use crate::env::ResolvedRoots;
use crate::model::{
    BaselineCheckItem, BaselineCheckReport, BaselineTarget, ConfigDocumentEntry, ConfigLoadError,
    ConfigScopeFile, Context, DocumentSource, DocumentStatus, Scope,
};
use crate::paths::normalize_path;

pub fn check_builtin_baseline(
    target: BaselineTarget,
    roots: &ResolvedRoots,
    strict: bool,
) -> Result<BaselineCheckReport, ConfigLoadError> {
    let mut items = builtin_items_for_target(target, roots);
    let builtin_keys: HashSet<BaselineKey> = items.iter().map(BaselineKey::from_item).collect();

    let configs = load_configs_from_roots(roots)?;
    let configs_in_load_order = configs.in_load_order();
    items.extend(required_extension_items(
        target,
        roots,
        &configs_in_load_order,
        &builtin_keys,
    ));

    let suggested_actions = suggested_actions(&items);

    Ok(BaselineCheckReport::from_items(
        target,
        strict,
        roots.codex_home.clone(),
        roots.project_path.clone(),
        items,
        suggested_actions,
    ))
}

fn builtin_items_for_target(
    target: BaselineTarget,
    roots: &ResolvedRoots,
) -> Vec<BaselineCheckItem> {
    let mut items = Vec::new();
    match target {
        BaselineTarget::Home => items.extend(home_items(roots)),
        BaselineTarget::Project => items.extend(project_items(roots)),
        BaselineTarget::All => {
            items.extend(home_items(roots));
            items.extend(project_items(roots));
        }
    }
    items
}

fn home_items(roots: &ResolvedRoots) -> Vec<BaselineCheckItem> {
    vec![
        startup_policy_item(Scope::Home, &roots.codex_home),
        required_item(
            Scope::Home,
            Context::SkillDev,
            "skill-dev",
            &roots.codex_home,
            "DEVELOPMENT.md",
            "skill development guidance from CODEX_HOME/DEVELOPMENT.md",
            DocumentSource::Builtin,
        ),
        required_item(
            Scope::Home,
            Context::TaskTools,
            "task-tools",
            &roots.codex_home,
            "CLI_TOOLS.md",
            "tool-selection guidance from CODEX_HOME/CLI_TOOLS.md",
            DocumentSource::Builtin,
        ),
    ]
}

fn project_items(roots: &ResolvedRoots) -> Vec<BaselineCheckItem> {
    vec![
        startup_policy_item(Scope::Project, &roots.project_path),
        required_item(
            Scope::Project,
            Context::ProjectDev,
            "project-dev",
            &roots.project_path,
            "DEVELOPMENT.md",
            "project development guidance from PROJECT_PATH/DEVELOPMENT.md",
            DocumentSource::Builtin,
        ),
    ]
}

fn startup_policy_item(scope: Scope, root: &Path) -> BaselineCheckItem {
    let override_path = normalize_path(&root.join("AGENTS.override.md"));
    if override_path.exists() {
        return BaselineCheckItem {
            scope,
            context: Context::Startup,
            label: "startup policy".to_string(),
            path: override_path,
            required: true,
            status: DocumentStatus::Present,
            source: DocumentSource::Builtin,
            why: format!(
                "startup {} policy (AGENTS.override.md preferred over AGENTS.md)",
                scope
            ),
        };
    }

    let fallback_path = normalize_path(&root.join("AGENTS.md"));
    let status = if fallback_path.exists() {
        DocumentStatus::Present
    } else {
        DocumentStatus::Missing
    };

    BaselineCheckItem {
        scope,
        context: Context::Startup,
        label: "startup policy".to_string(),
        path: fallback_path,
        required: true,
        status,
        source: DocumentSource::BuiltinFallback,
        why: format!(
            "startup {} policy (AGENTS.override.md missing, fallback AGENTS.md)",
            scope
        ),
    }
}

fn required_item(
    scope: Scope,
    context: Context,
    label: &str,
    root: &Path,
    file_name: &str,
    why: &str,
    source: DocumentSource,
) -> BaselineCheckItem {
    let path = normalize_path(&root.join(file_name));
    let status = if path.exists() {
        DocumentStatus::Present
    } else {
        DocumentStatus::Missing
    };

    BaselineCheckItem {
        scope,
        context,
        label: label.to_string(),
        path,
        required: true,
        status,
        source,
        why: why.to_string(),
    }
}

fn required_extension_items(
    target: BaselineTarget,
    roots: &ResolvedRoots,
    configs_in_load_order: &[&ConfigScopeFile],
    builtin_keys: &HashSet<BaselineKey>,
) -> Vec<BaselineCheckItem> {
    let mut extension_items = Vec::new();
    let mut extension_indices: HashMap<BaselineKey, usize> = HashMap::new();

    for config in configs_in_load_order {
        merge_required_extension_items(
            target,
            roots,
            config,
            builtin_keys,
            &mut extension_items,
            &mut extension_indices,
        );
    }

    extension_items
}

fn merge_required_extension_items(
    target: BaselineTarget,
    roots: &ResolvedRoots,
    config: &ConfigScopeFile,
    builtin_keys: &HashSet<BaselineKey>,
    extension_items: &mut Vec<BaselineCheckItem>,
    extension_indices: &mut HashMap<BaselineKey, usize>,
) {
    for (index, entry) in config.documents.iter().enumerate() {
        if !entry.required || !target_includes_scope(target, entry.scope) {
            continue;
        }

        let path = resolve_extension_path(entry, roots);
        let key = BaselineKey::new(entry.context, entry.scope, path.clone());
        if builtin_keys.contains(&key) {
            continue;
        }

        let item = BaselineCheckItem {
            scope: entry.scope,
            context: entry.context,
            label: entry.context.as_str().to_string(),
            path: path.clone(),
            required: true,
            status: if path.exists() {
                DocumentStatus::Present
            } else {
                DocumentStatus::Missing
            },
            source: extension_source(config.source_scope),
            why: extension_why(config, index, entry),
        };

        if let Some(existing_index) = extension_indices.get(&key).copied() {
            extension_items[existing_index] = item;
        } else {
            let next_index = extension_items.len();
            extension_items.push(item);
            extension_indices.insert(key, next_index);
        }
    }
}

fn target_includes_scope(target: BaselineTarget, scope: Scope) -> bool {
    match target {
        BaselineTarget::Home => scope == Scope::Home,
        BaselineTarget::Project => scope == Scope::Project,
        BaselineTarget::All => true,
    }
}

fn extension_source(source_scope: Scope) -> DocumentSource {
    match source_scope {
        Scope::Home => DocumentSource::ExtensionHome,
        Scope::Project => DocumentSource::ExtensionProject,
    }
}

fn resolve_extension_path(entry: &ConfigDocumentEntry, roots: &ResolvedRoots) -> PathBuf {
    let root = match entry.scope {
        Scope::Home => &roots.codex_home,
        Scope::Project => &roots.project_path,
    };
    normalize_path(&root.join(&entry.path))
}

fn extension_why(config: &ConfigScopeFile, index: usize, entry: &ConfigDocumentEntry) -> String {
    match entry
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|notes| !notes.is_empty())
    {
        Some(notes) => format!(
            "extension document from {} document[{index}] notes={notes}",
            config.file_path.display()
        ),
        None => format!(
            "extension document from {} document[{index}]",
            config.file_path.display()
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BaselineKey {
    context: &'static str,
    scope: &'static str,
    path: PathBuf,
}

impl BaselineKey {
    fn new(context: Context, scope: Scope, path: PathBuf) -> Self {
        Self {
            context: context.as_str(),
            scope: scope.as_str(),
            path,
        }
    }

    fn from_item(item: &BaselineCheckItem) -> Self {
        Self::new(item.context, item.scope, item.path.clone())
    }
}

fn suggested_actions(items: &[BaselineCheckItem]) -> Vec<String> {
    let has_home_missing_required = items.iter().any(|item| {
        item.scope == Scope::Home && item.required && matches!(item.status, DocumentStatus::Missing)
    });
    let has_project_missing_required = items.iter().any(|item| {
        item.scope == Scope::Project
            && item.required
            && matches!(item.status, DocumentStatus::Missing)
    });

    let mut actions = Vec::new();
    if has_home_missing_required {
        actions.push("agent-docs scaffold-baseline --missing-only --target home".to_string());
    }
    if has_project_missing_required {
        actions.push("agent-docs scaffold-baseline --missing-only --target project".to_string());
    }

    actions
}
