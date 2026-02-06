use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::load_configs_from_roots;
use crate::env::ResolvedRoots;
use crate::model::{
    ConfigDocumentEntry, ConfigLoadError, ConfigScopeFile, Context, DocumentSource, DocumentStatus,
    LoadedConfigs, ResolveReport, ResolveSummary, ResolvedDocument, Scope, SUPPORTED_CONTEXTS,
};
use crate::paths::normalize_path;

pub fn supported_contexts() -> &'static [Context] {
    &SUPPORTED_CONTEXTS
}

pub fn resolve(
    context: Context,
    roots: &ResolvedRoots,
    strict: bool,
) -> Result<ResolveReport, ConfigLoadError> {
    let configs = load_configs_from_roots(roots)?;
    Ok(resolve_with_configs(context, roots, strict, &configs))
}

pub fn resolve_builtin(context: Context, roots: &ResolvedRoots, strict: bool) -> ResolveReport {
    match resolve(context, roots, strict) {
        Ok(report) => report,
        Err(_) => resolve_builtin_only(context, roots, strict),
    }
}

pub fn resolve_builtin_only(
    context: Context,
    roots: &ResolvedRoots,
    strict: bool,
) -> ResolveReport {
    let documents = match context {
        Context::Startup => resolve_startup(roots),
        Context::SkillDev => vec![resolve_required_doc(
            Context::SkillDev,
            Scope::Home,
            &roots.codex_home,
            "DEVELOPMENT.md",
            "skill development guidance from CODEX_HOME/DEVELOPMENT.md",
            DocumentSource::Builtin,
        )],
        Context::TaskTools => vec![resolve_required_doc(
            Context::TaskTools,
            Scope::Home,
            &roots.codex_home,
            "CLI_TOOLS.md",
            "tool-selection guidance from CODEX_HOME/CLI_TOOLS.md",
            DocumentSource::Builtin,
        )],
        Context::ProjectDev => vec![resolve_required_doc(
            Context::ProjectDev,
            Scope::Project,
            &roots.project_path,
            "DEVELOPMENT.md",
            "project development guidance from PROJECT_PATH/DEVELOPMENT.md",
            DocumentSource::Builtin,
        )],
    };

    let summary = ResolveSummary::from_documents(&documents);

    ResolveReport {
        context,
        strict,
        codex_home: roots.codex_home.clone(),
        project_path: roots.project_path.clone(),
        documents,
        summary,
    }
}

pub fn resolve_with_configs(
    context: Context,
    roots: &ResolvedRoots,
    strict: bool,
    configs: &LoadedConfigs,
) -> ResolveReport {
    let mut documents = resolve_builtin_only(context, roots, strict).documents;
    let builtin_keys: HashSet<ResolveKey> =
        documents.iter().map(ResolveKey::from_document).collect();

    let mut extension_documents: Vec<ResolvedDocument> = Vec::new();
    let mut extension_indices: HashMap<ResolveKey, usize> = HashMap::new();

    for config in configs.in_load_order() {
        merge_extension_documents(
            context,
            roots,
            config,
            &builtin_keys,
            &mut extension_documents,
            &mut extension_indices,
        );
    }

    documents.extend(extension_documents);
    let summary = ResolveSummary::from_documents(&documents);

    ResolveReport {
        context,
        strict,
        codex_home: roots.codex_home.clone(),
        project_path: roots.project_path.clone(),
        documents,
        summary,
    }
}

fn merge_extension_documents(
    context: Context,
    roots: &ResolvedRoots,
    config: &ConfigScopeFile,
    builtin_keys: &HashSet<ResolveKey>,
    extension_documents: &mut Vec<ResolvedDocument>,
    extension_indices: &mut HashMap<ResolveKey, usize>,
) {
    for (index, entry) in config.documents.iter().enumerate() {
        if entry.context != context {
            continue;
        }

        let resolved_path = resolve_extension_path(entry, roots);
        let key = ResolveKey::new(context, entry.scope, resolved_path.clone());
        if builtin_keys.contains(&key) {
            continue;
        }

        let document = ResolvedDocument {
            context,
            scope: entry.scope,
            path: resolved_path.clone(),
            required: entry.required,
            status: if resolved_path.exists() {
                DocumentStatus::Present
            } else {
                DocumentStatus::Missing
            },
            source: extension_source(config.source_scope),
            why: extension_why(config, index, entry),
        };

        if let Some(existing_index) = extension_indices.get(&key).copied() {
            extension_documents[existing_index] = document;
        } else {
            let next_index = extension_documents.len();
            extension_documents.push(document);
            extension_indices.insert(key, next_index);
        }
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
struct ResolveKey {
    context: &'static str,
    scope: &'static str,
    path: PathBuf,
}

impl ResolveKey {
    fn new(context: Context, scope: Scope, path: PathBuf) -> Self {
        Self {
            context: context.as_str(),
            scope: scope.as_str(),
            path,
        }
    }

    fn from_document(document: &ResolvedDocument) -> Self {
        Self::new(document.context, document.scope, document.path.clone())
    }
}

fn resolve_startup(roots: &ResolvedRoots) -> Vec<ResolvedDocument> {
    vec![
        resolve_startup_scope(Scope::Home, &roots.codex_home),
        resolve_startup_scope(Scope::Project, &roots.project_path),
    ]
}

fn resolve_startup_scope(scope: Scope, root: &Path) -> ResolvedDocument {
    let override_path = normalize_path(&root.join("AGENTS.override.md"));
    if override_path.exists() {
        return ResolvedDocument {
            context: Context::Startup,
            scope,
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

    resolve_required_doc(
        Context::Startup,
        scope,
        root,
        "AGENTS.md",
        &format!(
            "startup {} policy (AGENTS.override.md missing, fallback AGENTS.md)",
            scope
        ),
        DocumentSource::BuiltinFallback,
    )
}

fn resolve_required_doc(
    context: Context,
    scope: Scope,
    root: &Path,
    file_name: &str,
    why: &str,
    source: DocumentSource,
) -> ResolvedDocument {
    let path = normalize_path(&root.join(file_name));
    let status = if path.exists() {
        DocumentStatus::Present
    } else {
        DocumentStatus::Missing
    };

    ResolvedDocument {
        context,
        scope,
        path,
        required: true,
        status,
        source,
        why: why.to_string(),
    }
}
