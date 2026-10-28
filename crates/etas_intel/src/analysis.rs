use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisPhase {
    #[default]
    Idle,
    Queued,
    Preparing,
    Checking,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisStatus {
    pub sequence: u64,
    pub phase: AnalysisPhase,
    pub project_root: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub files: usize,
    pub errors: usize,
    pub warnings: usize,
    pub message: Option<String>,
}
