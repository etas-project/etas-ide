use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};

use etas_core::{SourceId, TextSize};
use etas_frontend::{
    CheckMode, CheckRequest, CheckScope, DefinitionTarget, DiskCacheAccess, EntryPolicy,
    ExternalPackageId, FrontendSession, ImportTarget, MemoryArtifactReuse, ProjectChangeSet,
    ProjectCompileOptions, ProjectEnvironmentInput, ProjectInput, ProjectRevision,
    ProjectSemanticSnapshot, ProjectSessionId, ResolvedModuleTarget, SnapshotDetailLevel,
    SourceChange, SourceInput, SourceKind, SourceVersion,
};
use etas_hir::{HirNodeRef, HirType, ResolveResult, SyntaxNodeRef};

use crate::types::completion::{CompletionItem, CompletionParams};
use crate::types::diagnostic::IntelDiagnostic;
use crate::types::location::IntelLocation;

struct ProjectFile {
    path: PathBuf,
    source_id: SourceId,
}

pub(crate) struct DocumentState {
    pub(crate) uri: String,
    pub(crate) text: String,
    pub(crate) version: i32,
    pub(crate) source_id: SourceId,
}

struct PreparedPackageState {
    root: PathBuf,
    source_root: PathBuf,
    selected_bin: Option<etas_package::BinTarget>,
    environment: ProjectEnvironmentInput,
    package_environment: etas_package::PackageEnvironmentMetadata,
}

struct PendingPackagePrepare {
    rx: Receiver<Result<PreparedPackageState, String>>,
}

#[derive(Clone, Copy, Debug)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Clone, Debug)]
pub struct LspTextEdit {
    pub range: Option<LspRange>,
    pub text: String,
}

const DEBOUNCE_MS: u64 = 500;

pub type DiagnosticSnapshot = (
    HashMap<String, Vec<IntelDiagnostic>>,
    HashMap<SourceId, String>,
);

pub enum Command {
    SetWorkspaceRoot(PathBuf),
    DebouncedCheck,
    ReloadPackage,
    OpenDocument {
        uri: String,
        text: String,
        version: i32,
    },
    ChangeDocument {
        uri: String,
        content_changes: Vec<LspTextEdit>,
        version: i32,
    },
    CloseDocument {
        uri: String,
    },
    AddSourceFile {
        path: String,
        respond: tokio::sync::oneshot::Sender<()>,
    },
    RemoveSourceFile {
        path: String,
        respond: tokio::sync::oneshot::Sender<()>,
    },
    GetDiagnostics {
        respond: tokio::sync::oneshot::Sender<DiagnosticSnapshot>,
    },
    Hover {
        uri: String,
        position: LspPosition,
        respond: tokio::sync::oneshot::Sender<Option<HoverResult>>,
    },
    GoToDefinition {
        uri: String,
        position: LspPosition,
        respond: tokio::sync::oneshot::Sender<Vec<IntelLocation>>,
    },
    Completion {
        params: CompletionParams,
        respond: tokio::sync::oneshot::Sender<Vec<CompletionItem>>,
    },
    SemanticTokens {
        uri: String,
        respond: tokio::sync::oneshot::Sender<(Vec<IntelSemanticToken>, String)>,
    },
    DocumentSymbols {
        uri: String,
        respond: tokio::sync::oneshot::Sender<(Vec<crate::DocumentSymbol>, String)>,
    },
}

pub enum Event {
    DiagnosticsUpdated {
        diagnostics: HashMap<String, Vec<IntelDiagnostic>>,
        source_texts: HashMap<SourceId, String>,
        versions: HashMap<String, i32>,
    },
}

pub struct HoverResult {
    pub text: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub source_text: String,
}

pub struct IntelSemanticToken {
    pub byte_start: u32,
    pub byte_end: u32,
    pub token_type: u32,
    pub modifier_bits: u32,
}

struct LanguageService {
    session: FrontendSession,
    project_id: Option<ProjectSessionId>,
    documents: HashMap<String, DocumentState>,
    url_to_source_id: HashMap<String, SourceId>,
    next_source_id: u32,
    project_revision: u64,
    source_versions: HashMap<SourceId, u64>,
    workspace_root: Option<PathBuf>,
    snapshot: Option<ProjectSemanticSnapshot>,
    diagnostics: HashMap<String, Vec<IntelDiagnostic>>,
    pending_changes: Vec<SourceChange>,
    project_files: Vec<ProjectFile>,
    package_state: Option<PreparedPackageState>,
    pending_package_prepare: Option<PendingPackagePrepare>,
    package_error: Option<String>,
    analysis_stale: bool,
    timer_tx: Option<Sender<()>>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<Event>>,
}

impl LanguageService {
    fn new() -> Self {
        Self {
            session: FrontendSession::new(),
            project_id: None,
            documents: HashMap::new(),
            url_to_source_id: HashMap::new(),
            next_source_id: 1,
            project_revision: 0,
            source_versions: HashMap::new(),
            workspace_root: None,
            snapshot: None,
            diagnostics: HashMap::new(),
            pending_changes: Vec::new(),
            project_files: Vec::new(),
            package_state: None,
            pending_package_prepare: None,
            package_error: None,
            analysis_stale: false,
            timer_tx: None,
            event_tx: None,
        }
    }

    fn set_workspace_root(&mut self, path: PathBuf) {
        self.workspace_root = Some(canonical_or_original(path));
    }

    /// Discover .es files under src/ — only records paths, no content reading.
    fn discover_project_files(&mut self) {
        let src_dir = match self
            .package_state
            .as_ref()
            .map(|package| package.source_root.clone())
        {
            Some(dir) => dir,
            None => {
                let root = match &self.workspace_root {
                    Some(r) => r,
                    None => return,
                };
                default_source_root(root)
            }
        };
        if !src_dir.exists() || !src_dir.is_dir() {
            return;
        }

        let mut paths = Vec::new();
        collect_es_files(&src_dir, &mut paths);
        paths.sort();

        for raw_path in paths {
            let path = raw_path.canonicalize().unwrap_or(raw_path);
            if self.project_files.iter().any(|f| f.path == path) {
                continue;
            }
            let source_id = SourceId(self.next_source_id);
            self.next_source_id += 1;
            let uri = path_to_uri(&path);
            self.url_to_source_id.insert(uri, source_id);
            self.project_files.push(ProjectFile { path, source_id });
        }
    }

    fn open_document(&mut self, uri: String, text: String, version: i32) {
        let path = uri_to_path(&uri);

        if let Some(path) = &path {
            if !self.document_belongs_to_current_project(path) {
                if let Some(root) = package_root_for_document(path) {
                    self.switch_project_root(root);
                }
            }
        }

        // Ensure project is initialized before assigning source_id —
        // this prevents duplicate SourceId for already-discovered files.
        if self.project_id.is_none() {
            if let Some(path) = &path {
                self.infer_workspace_root_from_document(path);
            }
            self.ensure_project();
        }

        // Check if this file is already known from discovery
        let existing_id = self
            .project_files
            .iter()
            .find(|pf| pf.path == *path.as_ref().unwrap_or(&PathBuf::new()))
            .map(|pf| pf.source_id);

        let source_id = existing_id.unwrap_or_else(|| {
            let id = SourceId(self.next_source_id);
            self.next_source_id += 1;
            id
        });

        let mut added_project_file = false;
        if existing_id.is_none() {
            if let Some(path) = &path {
                if self.workspace_root.is_some() && self.document_belongs_to_current_project(path) {
                    self.project_files.push(ProjectFile {
                        path: path.clone(),
                        source_id,
                    });
                    added_project_file = true;
                }
            }
        }

        self.url_to_source_id.insert(uri.clone(), source_id);
        self.documents.insert(
            uri.clone(),
            DocumentState {
                uri: uri.clone(),
                text,
                version,
                source_id,
            },
        );

        self.source_versions.entry(source_id).or_insert(0);

        if added_project_file {
            self.rebuild_project();
            return;
        }

        // Opening an unchanged disk-backed document does not invalidate the
        // checked project. Re-publish its diagnostics with the editor version.
        if existing_id.is_some()
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.sources.files.iter().any(|source| {
                    source.id == source_id && source.text.as_ref() == self.documents[&uri].text
                })
            })
        {
            self.diagnostics.entry(uri.clone()).or_default();
            if let Some(event_tx) = &self.event_tx {
                let _ = event_tx.send(Event::DiagnosticsUpdated {
                    diagnostics: self.diagnostics.clone(),
                    source_texts: self.source_texts(),
                    versions: self
                        .documents
                        .values()
                        .map(|doc| (doc.uri.clone(), doc.version))
                        .collect(),
                });
            }
            return;
        }

        // Project always exists at this point
        if existing_id.is_some() {
            // File already in project, just replace text
            let doc = self.documents.get(&uri).unwrap();
            let version = self.source_versions[&source_id] + 1;
            self.source_versions.insert(source_id, version);
            self.pending_changes.push(SourceChange::Replace {
                source: source_id,
                version: SourceVersion(version),
                text: doc.text.clone(),
            });
        } else {
            let doc = self.documents.get(&uri).unwrap();
            let kind = if self.workspace_root.is_some() {
                SourceKind::SourceProjectFile
            } else {
                SourceKind::LspOverlay
            };
            self.pending_changes.push(SourceChange::Add(SourceInput {
                id: source_id,
                path,
                text: doc.text.clone(),
                kind,
            }));
        }

        self.run_check();
    }

    fn change_document(&mut self, uri: String, content_changes: Vec<LspTextEdit>, version: i32) {
        self.activate_document_project(&uri);

        let Some(doc) = self.documents.get_mut(&uri) else {
            tracing::warn!("change_document for unknown uri: {uri}");
            return;
        };

        let source_id = doc.source_id;
        let current_version = self.source_versions.get(&source_id).copied().unwrap_or(0);
        let new_version = current_version + 1;

        if content_changes.len() == 1 && content_changes[0].range.is_none() {
            doc.text = content_changes[0].text.clone();
        } else {
            for change in &content_changes {
                if let Some(range) = &change.range {
                    let start = offset_of_position(&doc.text, range.start);
                    let end = offset_of_position(&doc.text, range.end);
                    doc.text.replace_range(start..end, &change.text);
                } else {
                    doc.text = change.text.clone();
                }
            }
        }
        let source_change = SourceChange::Replace {
            source: source_id,
            version: SourceVersion(new_version),
            text: doc.text.clone(),
        };
        doc.version = version;

        self.pending_changes.retain(
            |change| !matches!(change, SourceChange::Replace { source, .. } if *source == source_id),
        );
        self.pending_changes.push(source_change);
        self.source_versions.insert(source_id, new_version);
        self.analysis_stale = true;
        if let Some(timer_tx) = &self.timer_tx {
            let _ = timer_tx.send(());
        } else {
            self.run_check();
        }
    }

    fn close_document(&mut self, uri: String) {
        let Some(doc) = self.documents.remove(&uri) else {
            return;
        };
        let is_project_file = self
            .project_files
            .iter()
            .any(|pf| pf.source_id == doc.source_id);
        if !is_project_file {
            if self.project_id.is_some() && self.source_versions.contains_key(&doc.source_id) {
                self.pending_changes
                    .push(SourceChange::Remove(doc.source_id));
                self.source_versions.remove(&doc.source_id);
                self.run_check();
            }
            self.url_to_source_id.remove(&uri);
        } else if let Some(path) = uri_to_path(&uri) {
            if let Ok(disk_text) = std::fs::read_to_string(path) {
                let current_version = self
                    .source_versions
                    .get(&doc.source_id)
                    .copied()
                    .unwrap_or(0);
                let new_version = current_version + 1;
                self.pending_changes.push(SourceChange::Replace {
                    source: doc.source_id,
                    version: SourceVersion(new_version),
                    text: disk_text,
                });
                self.source_versions.insert(doc.source_id, new_version);
                self.run_check();
            }
        }
        // Don't remove project files — they still exist on disk and remain jump targets.
        self.diagnostics.remove(&uri);
    }

    fn add_source_file(&mut self, _path_str: String) {
        let known_files = self.project_files.len();
        self.discover_project_files();
        if self.project_id.is_some() && self.project_files.len() != known_files {
            self.rebuild_project();
            return;
        }
        if self.project_id.is_some() {
            // Find newly added files and push Add changes
            for pf in &self.project_files {
                let already_known = self.source_versions.contains_key(&pf.source_id);
                if !already_known {
                    let text = std::fs::read_to_string(&pf.path).unwrap_or_default();
                    let uri = path_to_uri(&pf.path);
                    self.url_to_source_id.insert(uri, pf.source_id);
                    self.source_versions.insert(pf.source_id, 0);
                    self.pending_changes.push(SourceChange::Add(SourceInput {
                        id: pf.source_id,
                        path: Some(pf.path.clone()),
                        text,
                        kind: SourceKind::SourceProjectFile,
                    }));
                }
            }
        }
        self.run_check();
    }

    fn remove_source_file(&mut self, path_str: String) {
        let raw_path = PathBuf::from(&path_str);
        let path = raw_path.canonicalize().unwrap_or(raw_path);
        if let Some(idx) = self.project_files.iter().position(|f| f.path == path) {
            let uri = path_to_uri(&path);
            self.documents.remove(&uri);
            self.url_to_source_id.remove(&uri);
            self.diagnostics.remove(&uri);
            self.project_files.remove(idx);
            self.rebuild_project();
            return;
        }
        self.run_check();
    }

    fn get_diagnostics(&self) -> &HashMap<String, Vec<IntelDiagnostic>> {
        &self.diagnostics
    }

    fn infer_workspace_root_from_document(&mut self, path: &Path) {
        if self
            .current_source_root()
            .is_some_and(|src_dir| path.starts_with(src_dir))
        {
            return;
        }

        let Some(package_root) = package_root_for_document(path) else {
            return;
        };

        self.workspace_root = Some(canonical_or_original(package_root));
    }

    fn ensure_project(&mut self) {
        if self.project_id.is_some() {
            return;
        }

        self.poll_package_prepare();
        self.start_package_prepare_if_needed();
        if self.pending_package_prepare.is_some() || self.package_error.is_some() {
            return;
        }

        // Discover all .es files in workspace
        self.discover_project_files();

        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut sources = Vec::new();

        // Add all discovered project files
        for pf in &self.project_files {
            let text = if let Some(doc) = self
                .documents
                .values()
                .find(|d| uri_to_path(&d.uri).as_ref() == Some(&pf.path))
            {
                doc.text.clone()
            } else {
                std::fs::read_to_string(&pf.path).unwrap_or_default()
            };
            sources.push(SourceInput {
                id: pf.source_id,
                path: Some(pf.path.clone()),
                text,
                kind: SourceKind::SourceProjectFile,
            });
            seen.insert(pf.path.clone());
            // Track all source_ids in the project so AddSourceFile won't re-add them
            self.source_versions.entry(pf.source_id).or_insert(0);
        }

        // Add open documents that aren't project files (temporary / external),
        // but never merge files from a different src/ package into this project.
        for doc in self.documents.values() {
            let path = uri_to_path(&doc.uri).unwrap_or_default();
            if !seen.contains(&path) {
                if self.workspace_root.is_some() && !self.document_belongs_to_current_project(&path)
                {
                    continue;
                }
                let kind = if self.workspace_root.is_some() {
                    SourceKind::SourceProjectFile
                } else {
                    SourceKind::LspOverlay
                };
                sources.push(SourceInput {
                    id: doc.source_id,
                    path: Some(path.clone()),
                    text: doc.text.clone(),
                    kind,
                });
                seen.insert(path);
            }
        }

        let project_root = self
            .package_state
            .as_ref()
            .map(|package| package.root.clone())
            .or_else(|| self.workspace_root.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let source_root = self
            .package_state
            .as_ref()
            .map(|package| package.source_root.clone())
            .or_else(|| workspace_src_dir(&Some(project_root.clone())));
        let package_entry = self
            .package_state
            .as_ref()
            .and_then(|package| package.selected_bin.as_ref());
        let entry_module = package_entry.map(|bin| etas_frontend::ModulePath {
            segments: bin.module.split('.').map(str::to_owned).collect(),
        });
        let entry_flow = package_entry
            .map(|bin| bin.flow.clone())
            .unwrap_or_else(|| "main".to_owned());
        let environment = self
            .package_state
            .as_ref()
            .map(|package| package.environment.clone())
            .unwrap_or_default();
        if let Some(source_root) = &source_root {
            self.append_dependency_source_overlays(
                &project_root,
                source_root,
                &mut sources,
                &mut seen,
            );
        }

        let input = ProjectInput {
            project_root,
            source_root,
            options: ProjectCompileOptions {
                entry_policy: EntryPolicy::CompileOnly,
            },
            environment,
            sources,
            entry: etas_frontend::ProjectEntry {
                module: entry_module,
                flow: entry_flow,
            },
        };

        let id = self.session.open_project(input);
        self.project_id = Some(id);
        self.analysis_stale = true;
    }

    fn append_dependency_source_overlays(
        &mut self,
        project_root: &Path,
        project_source_root: &Path,
        sources: &mut Vec<SourceInput>,
        seen: &mut HashSet<PathBuf>,
    ) {
        let dependencies = match self
            .package_state
            .as_ref()
            .filter(|package| package.root == project_root)
        {
            Some(package) => package.package_environment.dependencies.clone(),
            None => return,
        };
        self.append_dependency_source_overlays_from_dependencies(
            project_root,
            project_source_root,
            &dependencies,
            sources,
            seen,
        );
    }

    fn append_dependency_source_overlays_from_dependencies(
        &mut self,
        base_package_root: &Path,
        project_source_root: &Path,
        dependencies: &[etas_package::ResolvedDependency],
        sources: &mut Vec<SourceInput>,
        seen: &mut HashSet<PathBuf>,
    ) {
        for dependency in dependencies {
            if let Some(package_root) =
                dependency_source_package_root(base_package_root, dependency)
            {
                if let Ok(manifest) = etas_package::read_manifest(&package_root) {
                    let source_root = package_root.join(&manifest.source.root);
                    self.append_dependency_source_overlay(
                        dependency,
                        project_source_root,
                        &source_root,
                        sources,
                        seen,
                    );
                    self.append_dependency_source_overlays_from_dependencies(
                        &package_root,
                        project_source_root,
                        &dependency.dependencies,
                        sources,
                        seen,
                    );
                }
            }
        }
    }

    fn append_dependency_source_overlay(
        &mut self,
        dependency: &etas_package::ResolvedDependency,
        project_source_root: &Path,
        dependency_source_root: &Path,
        sources: &mut Vec<SourceInput>,
        seen: &mut HashSet<PathBuf>,
    ) {
        let mut paths = Vec::new();
        collect_es_files(dependency_source_root, &mut paths);
        paths.sort();
        for path in paths {
            let Ok(relative_path) = path.strip_prefix(dependency_source_root) else {
                continue;
            };
            let virtual_path = project_source_root.join(relative_path);
            if !seen.insert(virtual_path.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let id = SourceId(self.next_source_id);
            self.next_source_id += 1;
            self.url_to_source_id.insert(path_to_uri(&path), id);
            sources.push(SourceInput {
                id,
                path: Some(virtual_path),
                text,
                kind: SourceKind::DependencySourceOverlay {
                    package: stable_external_package_id(dependency),
                    import_root: dependency.import_root.clone(),
                },
            });
            self.source_versions.entry(id).or_insert(0);
        }
    }

    fn document_belongs_to_current_project(&self, path: &Path) -> bool {
        let Some(src_dir) = self.current_source_root() else {
            let Some(workspace_root) = &self.workspace_root else {
                return self.project_id.is_none();
            };
            return package_root_for_document(path)
                .map(canonical_or_original)
                .is_some_and(|root| root == *workspace_root);
        };
        path.starts_with(src_dir)
    }

    fn current_source_root(&self) -> Option<PathBuf> {
        self.package_state
            .as_ref()
            .map(|package| package.source_root.clone())
            .or_else(|| {
                let root = self.workspace_root.as_ref()?;
                workspace_src_dir(&Some(root.clone()))
            })
    }

    fn switch_project_root(&mut self, root: PathBuf) {
        self.session = FrontendSession::new();
        self.project_id = None;
        self.url_to_source_id.clear();
        self.project_revision = 0;
        self.source_versions.clear();
        self.workspace_root = Some(canonical_or_original(root));
        self.snapshot = None;
        self.diagnostics.clear();
        self.pending_changes.clear();
        self.project_files.clear();
        self.package_state = None;
        self.pending_package_prepare = None;
        self.package_error = None;
    }

    fn activate_document_project(&mut self, uri: &str) -> bool {
        let Some(path) = uri_to_path(uri) else {
            return false;
        };

        let mut activated = false;
        if !self.document_belongs_to_current_project(&path) {
            if let Some(root) = package_root_for_document(&path) {
                self.switch_project_root(root);
                activated = true;
            }
        }
        if self.project_id.is_none() {
            self.infer_workspace_root_from_document(&path);
            self.ensure_project();
            activated = true;
        }

        let Some(source_id) = self
            .project_files
            .iter()
            .find(|pf| pf.path == path)
            .map(|pf| pf.source_id)
        else {
            return activated;
        };
        let Some(doc) = self.documents.get_mut(uri) else {
            return activated;
        };

        self.url_to_source_id.insert(uri.to_owned(), source_id);
        self.source_versions.entry(source_id).or_insert(0);
        if doc.source_id != source_id {
            doc.source_id = source_id;
        }
        activated
    }

    fn rebuild_project(&mut self) {
        self.session = FrontendSession::new();
        self.project_id = None;
        self.project_revision = 0;
        self.source_versions.clear();
        self.snapshot = None;
        self.pending_changes.clear();
        self.ensure_project();
        self.run_check();
    }

    fn start_package_prepare_if_needed(&mut self) {
        if self.package_state.is_some()
            || self.pending_package_prepare.is_some()
            || self.package_error.is_some()
        {
            return;
        }
        let Some(root) = self
            .workspace_root
            .clone()
            .and_then(|root| package_root(&root))
        else {
            return;
        };
        self.workspace_root = Some(canonical_or_original(root.clone()));

        let (tx, rx) = std::sync::mpsc::channel();
        let prepare_root = root.clone();
        std::thread::spawn(move || {
            let result = etas_package::prepare_package(etas_package::PreparePackageOptions {
                package_root: prepare_root,
                selected_bin: None,
            })
            .map_err(|error| error.to_string())
            .and_then(|prepared| {
                etas_driver::package_env::frontend_environment(prepared.environment.clone())
                    .map_err(|error| error.to_string())
                    .map(|environment| PreparedPackageState {
                        root: prepared.package_root,
                        source_root: prepared.source_root,
                        selected_bin: prepared.selected_bin,
                        environment,
                        package_environment: prepared.environment,
                    })
            });
            let _ = tx.send(result);
        });
        self.pending_package_prepare = Some(PendingPackagePrepare { rx });
        self.arm_debounced_check();
    }

    fn arm_debounced_check(&self) {
        if let Some(timer_tx) = &self.timer_tx {
            let _ = timer_tx.send(());
        }
    }

    fn poll_package_prepare(&mut self) -> bool {
        let Some(pending) = self.pending_package_prepare.take() else {
            return false;
        };
        match pending.rx.try_recv() {
            Ok(Ok(package)) => {
                tracing::info!(
                    "prepared etas package environment for {}",
                    package.root.display()
                );
                self.package_state = Some(package);
                true
            }
            Ok(Err(error)) => {
                tracing::warn!("failed to prepare etas package environment: {error}");
                self.package_error = Some(error);
                false
            }
            Err(TryRecvError::Empty) => {
                self.pending_package_prepare = Some(pending);
                false
            }
            Err(TryRecvError::Disconnected) => {
                tracing::warn!("package prepare worker disconnected");
                false
            }
        }
    }

    fn block_on_prepare(&mut self, timeout: std::time::Duration) -> bool {
        if self.package_state.is_some() {
            return false;
        }
        let Some(pending) = self.pending_package_prepare.take() else {
            return self.package_state.is_some();
        };
        match pending.rx.recv_timeout(timeout) {
            Ok(Ok(package)) => {
                tracing::info!(
                    "prepared etas package environment for {}",
                    package.root.display()
                );
                self.package_state = Some(package);
                true
            }
            Ok(Err(error)) => {
                tracing::warn!("failed to prepare etas package environment: {error}");
                self.package_error = Some(error);
                false
            }
            Err(RecvTimeoutError::Timeout) => {
                self.pending_package_prepare = Some(pending);
                false
            }
            Err(RecvTimeoutError::Disconnected) => {
                tracing::warn!("package prepare worker disconnected");
                false
            }
        }
    }

    fn run_check(&mut self) {
        if self.poll_package_prepare() {
            self.rebuild_project();
            return;
        }
        if self.package_error.is_some() {
            self.publish_unprepared_diagnostics();
            return;
        }

        let project_id = match self.project_id {
            Some(id) => id,
            None => {
                if self.package_error.is_some() {
                    self.publish_unprepared_diagnostics();
                }
                return;
            }
        };

        let changes = std::mem::take(&mut self.pending_changes);

        if !changes.is_empty() {
            self.project_revision += 1;
            let revision = self.project_revision;

            let change_set = ProjectChangeSet {
                revision: ProjectRevision(revision),
                source_changes: changes,
                dependency_overlay_changes: Vec::new(),
                environment_change: None,
                option_changes: vec![],
            };

            match self.session.apply_changes(project_id, change_set) {
                Ok(summary) => {
                    tracing::debug!(
                        "applied {} changes, revision={:?}",
                        summary.changed_sources.len(),
                        summary.revision
                    );
                }
                Err(e) => {
                    tracing::error!("apply_changes failed: {e:?}");
                    return;
                }
            }
        }

        let request = CheckRequest {
            mode: CheckMode::Incremental,
            scope: CheckScope::FullProject,
            disk_artifact_access: DiskCacheAccess::Disabled,
            memory_artifact_reuse: MemoryArtifactReuse::Enabled,
            snapshot_detail: SnapshotDetailLevel::Full,
            collect_pipeline_timing: false,
            retention: Default::default(),
        };

        match self.session.check(project_id, request) {
            Ok(response) => {
                tracing::info!(
                    "run_check: output.checked={} snapshot={} diagnostics={}",
                    response.output.checked.is_some(),
                    response.snapshot.is_some(),
                    response.output.diagnostics.len(),
                );
                if response.snapshot.is_some() {
                    self.snapshot = response.snapshot;
                }
                let mut new_diagnostics: HashMap<String, Vec<IntelDiagnostic>> = HashMap::new();

                for uri in self.documents.keys() {
                    new_diagnostics.entry(uri.clone()).or_default();
                }

                for diag in &response.output.diagnostics {
                    let source_id = diag.primary.span.source;
                    let uri = self.source_id_to_uri(source_id);

                    let intel_diag = IntelDiagnostic {
                        source_id,
                        range: diag.primary.span.range,
                        severity: diag.severity.into(),
                        code: format!("{:?}", diag.code),
                        message: diag.message.clone(),
                    };

                    new_diagnostics.entry(uri).or_default().push(intel_diag);
                }

                self.diagnostics = new_diagnostics;
                self.analysis_stale = false;
                if let Some(event_tx) = &self.event_tx {
                    let source_texts = self.source_texts();
                    let versions = self
                        .documents
                        .values()
                        .map(|doc| (doc.uri.clone(), doc.version))
                        .collect();
                    let _ = event_tx.send(Event::DiagnosticsUpdated {
                        diagnostics: self.diagnostics.clone(),
                        source_texts,
                        versions,
                    });
                }
            }
            Err(e) => {
                tracing::error!("check failed: {e:?}");
            }
        }
    }

    fn ensure_fresh_analysis(&mut self) {
        if self.poll_package_prepare() {
            self.rebuild_project();
            return;
        }
        if self.package_error.is_some() {
            self.publish_unprepared_diagnostics();
            return;
        }
        if self.pending_package_prepare.is_some() {
            self.arm_debounced_check();
            return;
        }
        if self.analysis_stale {
            self.run_check();
        }
    }

    fn publish_unprepared_diagnostics(&mut self) {
        let mut diagnostics = HashMap::new();
        for doc in self.documents.values() {
            let parsed = crate::syntax::parse(&doc.text);
            let mut items: Vec<_> = parsed
                .parsed
                .diagnostics
                .iter()
                .map(|diag| IntelDiagnostic {
                    source_id: doc.source_id,
                    range: diag.primary.span.range,
                    severity: diag.severity.into(),
                    code: format!("{:?}", diag.code),
                    message: diag.message.clone(),
                })
                .collect();
            if let Some(error) = &self.package_error {
                items.push(IntelDiagnostic {
                    source_id: doc.source_id,
                    range: etas_core::TextRange::new(TextSize(0), TextSize(0)),
                    severity: crate::IntelSeverity::Error,
                    code: "package::PrepareFailed".to_owned(),
                    message: format!("Package analysis unavailable: {error}"),
                });
            }
            diagnostics.insert(doc.uri.clone(), items);
        }
        self.diagnostics = diagnostics;
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(Event::DiagnosticsUpdated {
                diagnostics: self.diagnostics.clone(),
                source_texts: self.source_texts(),
                versions: self
                    .documents
                    .values()
                    .map(|doc| (doc.uri.clone(), doc.version))
                    .collect(),
            });
        }
    }

    fn source_id_to_uri(&self, source_id: SourceId) -> String {
        if let Some(doc) = self
            .documents
            .values()
            .find(|doc| doc.source_id == source_id)
        {
            return doc.uri.clone();
        }
        for (uri, id) in &self.url_to_source_id {
            if *id == source_id {
                return uri.clone();
            }
        }
        format!("file:///unknown_source_{}", source_id.0)
    }

    fn source_texts(&self) -> HashMap<SourceId, String> {
        let mut texts: HashMap<_, _> = self
            .snapshot
            .iter()
            .flat_map(|snapshot| snapshot.sources.files.iter())
            .map(|source| (source.id, source.text.to_string()))
            .collect();
        for file in &self.project_files {
            if let std::collections::hash_map::Entry::Vacant(entry) = texts.entry(file.source_id) {
                if let Ok(text) = std::fs::read_to_string(&file.path) {
                    entry.insert(text);
                }
            }
        }
        texts.extend(
            self.documents
                .values()
                .map(|doc| (doc.source_id, doc.text.clone())),
        );
        texts
    }

    fn hover(&self, uri: &str, position: LspPosition) -> Option<HoverResult> {
        let snapshot = self.snapshot.as_ref()?;
        let doc = self.documents.get(uri)?;
        crate::hover::hover(snapshot, doc, offset_of_position(&doc.text, position))
    }

    fn go_to_definition(&self, uri: &str, position: LspPosition) -> Vec<IntelLocation> {
        let source_id = match self.url_to_source_id.get(uri) {
            Some(id) => *id,
            None => return vec![],
        };
        let snapshot = match self.snapshot.as_ref() {
            Some(s) => s,
            None => return vec![],
        };
        let doc = match self.documents.get(uri) {
            Some(d) => d,
            None => return vec![],
        };

        let offset = offset_of_position(&doc.text, position);
        let target_offset = TextSize(offset as u32);
        if let Some(locations) =
            self.resolve_path_item_at_offset(source_id, offset, &doc.text, snapshot)
        {
            return locations;
        }
        let node = match self.find_node_at(source_id, target_offset, snapshot) {
            Some(n) => n,
            None => {
                return self
                    .resolve_import_prefix_at_offset(source_id, offset, &doc.text, snapshot);
            }
        };
        let hir_nodes = snapshot.hir_nodes_for_syntax(node.id);

        let mut locations = Vec::new();

        for hir_ref in hir_nodes {
            match *hir_ref {
                HirNodeRef::Expr(expr_id) => {
                    if let Some(target) = snapshot.definition_target_for_expr(expr_id) {
                        locations.extend(self.resolve_definition_target(target, snapshot));
                    }
                }
                HirNodeRef::Symbol(symbol_id) => {
                    if let Some(target) = snapshot.definition_target_for_symbol(symbol_id) {
                        locations.extend(self.resolve_definition_target(target, snapshot));
                    }
                }
                HirNodeRef::Item(item_id) => {
                    // First, check if the cursor is on a resolved import
                    // and resolve its target (jump to the imported module/item)
                    if let Some(origin) = snapshot.hir_origin(etas_hir::HirNodeRef::Item(item_id)) {
                        let import_span = origin.span();
                        let mut resolved_via_import = false;
                        for resolved in &snapshot.resolved_imports.imports {
                            if spans_overlap(import_span, resolved.import.span)
                                || spans_overlap(import_span, resolved.span)
                            {
                                locations.extend(self.resolve_definition_target(
                                    &DefinitionTarget::Import(resolved.target.clone()),
                                    snapshot,
                                ));
                                resolved_via_import = true;
                                break;
                            }
                        }
                        if !resolved_via_import {
                            locations.push(self.span_to_location(origin.span()));
                        }
                    }
                }
                HirNodeRef::Type(type_id) => {
                    if let Some(HirType::Path { path, .. }) = snapshot.hir.types.get(type_id) {
                        if let ResolveResult::Resolved(symbol_id) = path.resolution {
                            if let Some(target) = snapshot.definition_target_for_symbol(symbol_id) {
                                locations.extend(self.resolve_definition_target(target, snapshot));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if locations.is_empty() {
            self.resolve_import_prefix_at_offset(source_id, offset, &doc.text, snapshot)
        } else {
            locations
        }
    }

    fn completion(&self, params: CompletionParams) -> Vec<CompletionItem> {
        crate::completion::complete(crate::completion::CompletionRequest {
            documents: &self.documents,
            snapshot: self.snapshot.as_ref(),
            params,
        })
    }

    fn resolve_import_prefix_at_offset(
        &self,
        source_id: SourceId,
        offset: usize,
        text: &str,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Vec<IntelLocation> {
        let Some(prefix) = path_prefix_at_offset(text, offset) else {
            return vec![];
        };
        let Some(part_id) = snapshot
            .modules
            .parts
            .iter()
            .find_map(|(_, part)| (part.source == source_id).then_some(part.id))
        else {
            return vec![];
        };

        snapshot
            .resolved_imports
            .imports
            .iter()
            .find(|import| import.from_part == part_id && import.local_name == prefix)
            .map(|import| self.resolve_import_target(&import.target, snapshot))
            .or_else(|| {
                let module_path = etas_frontend::ModulePath {
                    segments: vec![prefix],
                };
                snapshot
                    .modules
                    .by_path
                    .get(&module_path)
                    .and_then(|module_id| snapshot.modules.modules.get(*module_id))
                    .map(|info| vec![self.span_to_location(info.span)])
            })
            .unwrap_or_default()
    }

    fn resolve_path_item_at_offset(
        &self,
        source_id: SourceId,
        offset: usize,
        text: &str,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Option<Vec<IntelLocation>> {
        let path = path_at_offset(text, offset)?;
        let segment = path.segments.get(path.segment_index)?;

        let part_id = snapshot
            .modules
            .parts
            .iter()
            .find_map(|(_, part)| (part.source == source_id).then_some(part.id))?;

        if path.segment_index == 0 {
            return self.resolve_wildcard_path_at_offset(source_id, offset, segment, snapshot);
        }

        if let Some(import) = snapshot.resolved_imports.imports.iter().find(|import| {
            import.from_part == part_id
                && import.local_name == *segment
                && span_contains_offset(import.import.span, source_id, offset)
        }) {
            let locations = self.resolve_import_target(&import.target, snapshot);
            if !locations.is_empty() {
                return Some(locations);
            }
        }

        if let Some(first_segment) = path.segments.first() {
            if let Some(import) =
                snapshot.resolved_imports.imports.iter().find(|import| {
                    import.from_part == part_id && import.local_name == *first_segment
                })
            {
                if let ImportTarget::Module(module) = &import.target {
                    if let Some(locations) = self.resolve_module_export(module, segment, snapshot) {
                        return Some(locations);
                    }
                }
            }

            let module_path = etas_frontend::ModulePath {
                segments: vec![first_segment.clone()],
            };
            if let Some(module_id) = snapshot.modules.by_path.get(&module_path) {
                if let Some(locations) =
                    self.resolve_source_module_export(*module_id, segment, snapshot)
                {
                    return Some(locations);
                }
            }
        }

        snapshot
            .resolved_paths
            .expr_paths
            .iter()
            .find(|resolved| span_contains_offset(resolved.path.span, source_id, offset))
            .and_then(|resolved| match &resolved.result {
                etas_frontend::HirPathResolution::PartialImport {
                    target: ImportTarget::Module(module),
                    remaining,
                    ..
                } if remaining.first() == Some(segment) => {
                    self.resolve_module_export(module, segment, snapshot)
                }
                _ => None,
            })
    }

    fn resolve_wildcard_path_at_offset(
        &self,
        source_id: SourceId,
        offset: usize,
        segment: &str,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Option<Vec<IntelLocation>> {
        snapshot
            .resolved_paths
            .expr_paths
            .iter()
            .find(|resolved| span_contains_offset(resolved.path.span, source_id, offset))
            .and_then(|resolved| match &resolved.result {
                etas_frontend::HirPathResolution::WildcardImport { target } => {
                    self.resolve_module_export(target, segment, snapshot)
                }
                etas_frontend::HirPathResolution::AmbiguousWildcard { targets } => {
                    let locations = targets
                        .iter()
                        .filter_map(|target| self.resolve_module_export(target, segment, snapshot))
                        .flatten()
                        .collect::<Vec<_>>();
                    (!locations.is_empty()).then_some(locations)
                }
                _ => None,
            })
    }

    fn resolve_module_export(
        &self,
        module: &ResolvedModuleTarget,
        name: &str,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Option<Vec<IntelLocation>> {
        match module {
            ResolvedModuleTarget::Source { module, .. } => {
                self.resolve_source_module_export(*module, name, snapshot)
            }
            ResolvedModuleTarget::Std { .. } | ResolvedModuleTarget::External { .. } => None,
        }
    }

    fn resolve_source_module_export(
        &self,
        module_id: etas_frontend::ModuleId,
        name: &str,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Option<Vec<IntelLocation>> {
        snapshot
            .modules
            .modules
            .get(module_id)
            .and_then(|module| module.visibility_exports.items.get(name))
            .map(|item| vec![self.span_to_location(item.item.span)])
    }

    fn resolve_definition_target(
        &self,
        target: &DefinitionTarget,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Vec<IntelLocation> {
        match target {
            DefinitionTarget::SourceSymbol { span, .. } => self
                .resolve_import_alias_span(*span, snapshot)
                .unwrap_or_else(|| vec![self.span_to_location(*span)]),
            DefinitionTarget::Import(import_target) => {
                self.resolve_import_target(import_target, snapshot)
            }
        }
    }

    fn resolve_import_alias_span(
        &self,
        span: etas_core::Span,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Option<Vec<IntelLocation>> {
        snapshot
            .resolved_imports
            .imports
            .iter()
            .find(|resolved| {
                spans_overlap(span, resolved.span) || spans_overlap(span, resolved.import.span)
            })
            .map(|resolved| self.resolve_import_target(&resolved.target, snapshot))
    }

    fn resolve_import_target(
        &self,
        target: &ImportTarget,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Vec<IntelLocation> {
        match target {
            ImportTarget::Module(resolved) => match resolved {
                ResolvedModuleTarget::Source { module, .. } => snapshot
                    .modules
                    .modules
                    .get(*module)
                    .map(|info| vec![self.span_to_location(info.span)])
                    .unwrap_or_default(),
                ResolvedModuleTarget::Std { .. } | ResolvedModuleTarget::External { .. } => {
                    vec![]
                }
            },
            ImportTarget::SourceItem { item, .. } => {
                // AstItemRef has a span directly
                vec![self.span_to_location(item.span)]
            }
            ImportTarget::StdItem { .. } | ImportTarget::ExternalItem { .. } => {
                vec![]
            }
        }
    }

    fn span_to_location(&self, span: etas_core::Span) -> IntelLocation {
        let uri = self.source_id_to_uri(span.source);
        let source_text = self
            .documents
            .get(&uri)
            .map(|doc| doc.text.clone())
            .or_else(|| uri_to_path(&uri).and_then(|path| std::fs::read_to_string(path).ok()))
            .unwrap_or_default();
        IntelLocation {
            uri,
            range: crate::syntax::definition_selection(
                &crate::syntax::parse(&source_text),
                span.range,
            ),
            source_text,
        }
    }

    fn find_node_at(
        &self,
        source_id: SourceId,
        offset: TextSize,
        snapshot: &ProjectSemanticSnapshot,
    ) -> Option<SyntaxNodeRef> {
        let mut best: Option<SyntaxNodeRef> = None;
        let mut best_len: u32 = u32::MAX;

        for node in snapshot.source_map.syntax_nodes.values() {
            if node.span.source != source_id {
                continue;
            }
            let start = node.span.range.start.0;
            let end = node.span.range.end.0;
            if offset.0 >= start && offset.0 <= end {
                let len = end - start;
                if len < best_len {
                    best_len = len;
                    best = Some(*node);
                }
            }
        }

        best
    }

    fn semantic_tokens(&self, uri: &str) -> (Vec<IntelSemanticToken>, String) {
        let Some(doc) = self.documents.get(uri) else {
            return (vec![], String::new());
        };
        let parsed = crate::syntax::parse(&doc.text);
        let lexical = crate::syntax::lexical_tokens(&parsed);
        let snapshot = match self.snapshot.as_ref() {
            Some(s) => s,
            None => return (lexical, doc.text.clone()),
        };
        let source_id = match self.url_to_source_id.get(uri) {
            Some(id) => *id,
            None => return (vec![], String::new()),
        };
        let source_text = snapshot
            .sources
            .files
            .iter()
            .find(|f| f.id == source_id)
            .map(|f| f.text.to_string())
            .unwrap_or_default();
        if source_text != doc.text {
            return (lexical, doc.text.clone());
        }
        let tokens = crate::semantic_tokens::collect(snapshot, source_id, &parsed);

        (
            crate::syntax::merge_tokens(&parsed, lexical, tokens),
            source_text,
        )
    }
}

pub(crate) fn offset_of_position(text: &str, pos: LspPosition) -> usize {
    let mut current_line: u32 = 0;
    let mut byte_offset: usize = 0;

    for (i, ch) in text.char_indices() {
        if current_line == pos.line {
            let mut char_offset: u32 = 0;
            for (j, c) in text[byte_offset..].char_indices() {
                if c == '\n' || char_offset >= pos.character {
                    return byte_offset + j;
                }
                char_offset += c.len_utf16() as u32;
            }
            return text.len();
        }
        if ch == '\n' {
            current_line += 1;
            byte_offset = i + 1;
        }
    }
    text.len()
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    Some(path.canonicalize().unwrap_or(path))
}

fn path_to_uri(path: &PathBuf) -> String {
    url::Url::from_file_path(path)
        .expect("project source path must be absolute")
        .to_string()
}

fn path_prefix_at_offset(text: &str, offset: usize) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let offset = offset.min(text.len());
    let mut start = offset;
    while start > 0 {
        let Some((prev_start, ch)) = text[..start].char_indices().next_back() else {
            break;
        };
        if is_path_char(ch) {
            start = prev_start;
        } else {
            break;
        }
    }

    let mut end = offset;
    while end < text.len() {
        let Some(ch) = text[end..].chars().next() else {
            break;
        };
        if is_path_char(ch) {
            end += ch.len_utf8();
        } else {
            break;
        }
    }

    let path = text[start..end].trim_matches('.');
    let prefix = path.split('.').next()?;
    (!prefix.is_empty()).then(|| prefix.to_owned())
}

struct PathAtOffset {
    segments: Vec<String>,
    segment_index: usize,
}

fn path_at_offset(text: &str, offset: usize) -> Option<PathAtOffset> {
    if text.is_empty() {
        return None;
    }
    let offset = offset.min(text.len());
    let mut start = offset;
    while start > 0 {
        let Some((prev_start, ch)) = text[..start].char_indices().next_back() else {
            break;
        };
        if is_path_char(ch) {
            start = prev_start;
        } else {
            break;
        }
    }

    let mut end = offset;
    while end < text.len() {
        let Some(ch) = text[end..].chars().next() else {
            break;
        };
        if is_path_char(ch) {
            end += ch.len_utf8();
        } else {
            break;
        }
    }

    let raw_path = text[start..end].trim_matches('.');
    if raw_path.is_empty() {
        return None;
    }

    let segments = raw_path
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }

    let relative = offset.saturating_sub(start).min(raw_path.len());
    let segment_index = raw_path[..relative]
        .bytes()
        .filter(|byte| *byte == b'.')
        .count()
        .min(segments.len() - 1);

    Some(PathAtOffset {
        segments,
        segment_index,
    })
}

fn is_path_char(ch: char) -> bool {
    ch == '_' || ch == '.' || ch.is_ascii_alphanumeric()
}

fn span_contains_offset(span: etas_core::Span, source_id: SourceId, offset: usize) -> bool {
    span.source == source_id
        && span.range.start.0 as usize <= offset
        && offset <= span.range.end.0 as usize
}

fn spans_overlap(left: etas_core::Span, right: etas_core::Span) -> bool {
    left.source == right.source
        && left.range.start.0 <= right.range.end.0
        && right.range.start.0 <= left.range.end.0
}

fn workspace_src_dir(root: &Option<PathBuf>) -> Option<PathBuf> {
    let root = root.as_ref()?;
    let src_dir = default_source_root(root);
    src_dir.is_dir().then_some(src_dir)
}

fn canonical_or_original(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn default_source_root(root: &std::path::Path) -> PathBuf {
    if root.file_name().and_then(|n| n.to_str()) == Some("src") {
        root.to_path_buf()
    } else {
        root.join("src")
    }
}

fn package_root(path: &std::path::Path) -> Option<PathBuf> {
    etas_package::discover_manifest(path).and_then(|manifest| manifest.parent().map(PathBuf::from))
}

fn package_root_for_document(path: &std::path::Path) -> Option<PathBuf> {
    if let Some(root) = package_root(path) {
        return Some(root);
    }

    let mut ancestor = path.parent()?;
    loop {
        if ancestor.file_name().and_then(|n| n.to_str()) == Some("src") {
            return ancestor.parent().map(PathBuf::from);
        }
        ancestor = ancestor.parent()?;
    }
}

fn dependency_source_package_root(
    base_package_root: &Path,
    dependency: &etas_package::ResolvedDependency,
) -> Option<PathBuf> {
    let path = match &dependency.source {
        etas_package::ResolvedDependencySource::Path { path, .. }
        | etas_package::ResolvedDependencySource::Vendor { path, .. } => PathBuf::from(path),
        _ => return None,
    };
    let path = if path.is_absolute() {
        path
    } else {
        base_package_root.join(path)
    };
    path.canonicalize().ok()
}

fn stable_external_package_id(dependency: &etas_package::ResolvedDependency) -> ExternalPackageId {
    let key = format!(
        "{}|{}|{}|{}",
        dependency.import_root,
        dependency.identity.name,
        dependency.identity.version,
        dependency.identity.edition
    );
    let digest = blake3::hash(key.as_bytes());
    let bytes = digest.as_bytes();
    ExternalPackageId(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn collect_es_files(dir: &std::path::Path, paths: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_es_files(&path, paths);
        } else if path.extension().and_then(|x| x.to_str()) == Some("es") {
            paths.push(path);
        }
    }
}

pub fn spawn_service_with_events() -> (
    tokio::sync::mpsc::UnboundedSender<Command>,
    tokio::sync::mpsc::UnboundedReceiver<Event>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Command>();
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let (arm_tx, arm_rx) = std::sync::mpsc::channel::<()>();

    // Debounce timer thread: each arm resets the deadline; fires once on quiescence.
    let timer_cmd_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            if arm_rx.recv().is_err() {
                break;
            }
            loop {
                match arm_rx.recv_timeout(std::time::Duration::from_millis(DEBOUNCE_MS)) {
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if timer_cmd_tx.send(Command::DebouncedCheck).is_err() {
                break;
            }
        }
    });

    // Compiler queries run on a dedicated worker, not the platform's smaller
    // default background-thread stack. Match a foreground compiler's 8 MiB stack.
    std::thread::Builder::new()
        .name("etas-analysis".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut service = LanguageService::new();
            service.timer_tx = Some(arm_tx);
            service.event_tx = Some(event_tx);

            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    Command::SetWorkspaceRoot(path) => {
                        service.set_workspace_root(path);
                    }
                    Command::DebouncedCheck => {
                        service.ensure_fresh_analysis();
                    }
                    Command::ReloadPackage => {
                        service.package_state = None;
                        service.package_error = None;
                        service.pending_package_prepare = None;
                        service.rebuild_project();
                    }
                    Command::OpenDocument { uri, text, version } => {
                        service.open_document(uri, text, version);
                    }
                    Command::ChangeDocument {
                        uri,
                        content_changes,
                        version,
                    } => {
                        service.change_document(uri, content_changes, version);
                    }
                    Command::CloseDocument { uri } => {
                        service.close_document(uri);
                    }
                    Command::AddSourceFile { path, respond } => {
                        service.add_source_file(path);
                        let _ = respond.send(());
                    }
                    Command::RemoveSourceFile { path, respond } => {
                        service.remove_source_file(path);
                        let _ = respond.send(());
                    }
                    Command::GetDiagnostics { respond } => {
                        if service.poll_package_prepare() {
                            service.rebuild_project();
                        }
                        service.ensure_fresh_analysis();
                        let diags = service.get_diagnostics().clone();
                        let texts = service.source_texts();
                        let _ = respond.send((diags, texts));
                    }
                    Command::Hover {
                        uri,
                        position,
                        respond,
                    } => {
                        service.activate_document_project(&uri);
                        if service.block_on_prepare(std::time::Duration::from_secs(30)) {
                            service.rebuild_project();
                        }
                        service.ensure_fresh_analysis();
                        let result = service.hover(&uri, position);
                        let _ = respond.send(result);
                    }
                    Command::GoToDefinition {
                        uri,
                        position,
                        respond,
                    } => {
                        service.activate_document_project(&uri);
                        if service.block_on_prepare(std::time::Duration::from_secs(30)) {
                            service.rebuild_project();
                        }
                        service.ensure_fresh_analysis();
                        let result = service.go_to_definition(&uri, position);
                        let _ = respond.send(result);
                    }
                    Command::Completion { params, respond } => {
                        service.activate_document_project(&params.uri);
                        if service.block_on_prepare(std::time::Duration::from_secs(30)) {
                            service.rebuild_project();
                        }
                        let result = service.completion(params);
                        let _ = respond.send(result);
                    }
                    Command::SemanticTokens { uri, respond } => {
                        service.activate_document_project(&uri);
                        service.ensure_fresh_analysis();
                        let result = service.semantic_tokens(&uri);
                        let _ = respond.send(result);
                    }
                    Command::DocumentSymbols { uri, respond } => {
                        let text = service
                            .documents
                            .get(&uri)
                            .map(|doc| doc.text.clone())
                            .unwrap_or_default();
                        let parsed = crate::syntax::parse(&text);
                        let _ = respond.send((crate::syntax::document_symbols(&parsed), text));
                    }
                }
            }
        })
        .expect("failed to start Etas analysis worker");

    (tx, event_rx)
}

pub fn spawn_service() -> tokio::sync::mpsc::UnboundedSender<Command> {
    let (tx, _event_rx) = spawn_service_with_events();
    tx
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "semantic_token_tests.rs"]
mod semantic_token_tests;
