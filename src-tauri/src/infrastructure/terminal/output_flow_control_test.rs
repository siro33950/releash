use super::*;

#[test]
fn test_出力制御_上限超過と下限未満で遷移する() {
    let mut flow = OutputFlowControl::default();
    flow.subscribe("client", 0);
    assert!(!flow.output(1, OUTPUT_HIGH_WATERMARK));
    assert!(flow.output(2, 1));
    assert!(flow.processed("client", OUTPUT_HIGH_WATERMARK + 1 - OUTPUT_LOW_WATERMARK));
    assert!(!flow.processed("client", 1));
}

#[test]
fn test_出力制御_最も遅い購読を使い停止時に解放する() {
    let mut flow = OutputFlowControl::default();
    flow.subscribe("fast", 0);
    flow.subscribe("slow", 0);
    assert!(flow.output(1, OUTPUT_HIGH_WATERMARK + 1));
    assert!(flow.processed("fast", OUTPUT_HIGH_WATERMARK + 1));
    assert!(flow.processed("unknown", usize::MAX));
    assert!(!flow.unsubscribe("slow"));
    assert!(OUTPUT_REPORT_UNITS <= OUTPUT_LOW_WATERMARK);
}

#[test]
fn test_出力制御_重複と逆順を二重計上せず再生成で番号をリセットする() {
    let mut flow = OutputFlowControl::default();
    flow.subscribe("client", 0);
    assert!(!flow.output(2, OUTPUT_HIGH_WATERMARK));
    assert!(!flow.output(2, 1));
    assert!(!flow.output(1, 1));
    flow.reset(0);
    assert!(flow.output(1, OUTPUT_HIGH_WATERMARK + 1));
}
