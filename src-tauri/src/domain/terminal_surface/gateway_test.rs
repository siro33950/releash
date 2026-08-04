use super::{TerminalSurfaceRepository, TerminalSurfaceSummary};

#[allow(dead_code)]
fn assert_repositoryがmetadataを返す(repository: &impl TerminalSurfaceRepository) {
    let _: Option<TerminalSurfaceSummary> = repository.find_summary_by_session_key("surface-key");
    let _: Vec<TerminalSurfaceSummary> = repository.list_summaries();
}
