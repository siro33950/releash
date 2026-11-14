use super::*;

fn report(items: Vec<DiagnosticItem>) -> DiagnosticReport {
    DiagnosticReport {
        items,
        workflow_summaries: HashMap::new(),
        facet_summaries: HashMap::new(),
        facet_usage: HashMap::new(),
    }
}

fn item(severity: Severity) -> DiagnosticItem {
    DiagnosticItem {
        code: "E001".into(),
        severity,
        stage: DiagnosticStage::Resolve,
        span: None,
        message: "diagnostic".into(),
        workflow_name: None,
        node_name: None,
        facet_key: None,
        facet_kind: None,
        field: None,
    }
}

#[test]
fn test_診断全体件数_診断がなければ両重大度ともゼロ() {
    // Given
    let report = report(vec![]);
    // When
    let total = report.total();
    // Then
    assert_eq!(
        total,
        DiagnosticSummary {
            error_count: 0,
            info_count: 0
        }
    );
}

#[test]
fn test_診断全体件数_重大度ごとに数え同じ内容の別診断も数える() {
    // Given
    let report = report(vec![
        item(Severity::Error),
        item(Severity::Info),
        item(Severity::Error),
    ]);
    // When
    let total = report.total();
    // Then
    assert_eq!(
        total,
        DiagnosticSummary {
            error_count: 2,
            info_count: 1
        }
    );
}

#[test]
fn test_診断全体件数_定義とfacetの両方に関わる診断を一度だけ数える() {
    // Given
    let mut diagnostic = item(Severity::Error);
    diagnostic.workflow_name = Some("workflow".into());
    diagnostic.facet_key = Some("policy".into());
    diagnostic.facet_kind = Some("policy".into());
    let mut report = report(vec![diagnostic]);
    let summary = DiagnosticSummary {
        error_count: 1,
        info_count: 0,
    };
    report
        .workflow_summaries
        .insert("workflow".into(), summary.clone());
    report
        .facet_summaries
        .insert("policy/policy".into(), summary.clone());
    // When
    let total = report.total();
    // Then
    assert_eq!(total, summary);
}
