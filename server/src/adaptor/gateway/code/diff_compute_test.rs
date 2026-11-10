use super::*;

#[test]
fn test_diff_途中の取消を空や部分結果へ変換しない() {
    use crate::common::operation_context::{Deadline, OperationContext};
    use crate::common::test_helpers::CancelAfter;
    use std::sync::{atomic::AtomicUsize, Arc};
    for after in [0, 2, 4, 6, 8] {
        let context = OperationContext::new(
            None,
            Arc::new(CancelAfter {
                remaining: AtomicUsize::new(after),
            }),
        );
        assert!(matches!(
            crate::common::operation_context::sync_scope(context, || diff_buffers(
                "a\nb\n", "c\nd\n", None
            )),
            Err(CodeError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    ..
                }
            ))
        ));
    }
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    assert!(matches!(
        crate::common::operation_context::sync_scope(context, || diff_buffers("a", "b", None)),
        Err(CodeError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                ..
            }
        ))
    ));
}

mod diff_compute_gateway_tests {

    use super::*;

    // ── diff_buffers（git2 diff） ──

    #[test]
    fn test_diff_同一内容は空() {
        let hunks = diff_buffers("hello\nworld\n", "hello\nworld\n", None).unwrap();
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_diff_追加行検出() {
        let hunks = diff_buffers("line1\nline2\n", "line1\nline2\nline3\n", None).unwrap();
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "+line3"));
    }

    #[test]
    fn test_diff_削除行検出() {
        let hunks = diff_buffers("line1\nline2\nline3\n", "line1\nline3\n", None).unwrap();
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "-line2"));
    }

    #[test]
    fn test_diff_変更行検出() {
        let hunks =
            diff_buffers("line1\noriginal\nline3\n", "line1\nmodified\nline3\n", None).unwrap();
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "-original"));
        assert!(hunks[0].lines.iter().any(|l| l == "+modified"));
    }

    #[test]
    fn test_diff_複数hunk検出() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let hunks = diff_buffers(lines, &modified, None).unwrap();
        assert_eq!(hunks.len(), 2);
    }

    #[test]
    fn test_diff_空original() {
        let hunks = diff_buffers("", "new content\n", None).unwrap();
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "+new content"));
    }

    #[test]
    fn test_diff_空modified() {
        let hunks = diff_buffers("content\n", "", None).unwrap();
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "-content"));
    }

    #[test]
    fn test_diff_hunkは位置情報を持つ() {
        let hunks = diff_buffers("line1\nline2\n", "line1\nline2\nline3\n", None).unwrap();
        assert!(hunks[0].old_start > 0);
        assert!(hunks[0].new_start > 0);
    }
}
