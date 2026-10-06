use super::*;

#[test]
fn test_物理回収判定_二つの閾値をともに満たす場合だけ発火する() {
    let page_size = 4096;
    let minimum_pages = MINIMUM_RECLAIM_BYTES / page_size;

    assert!(should_reclaim(FreelistStats {
        page_count: minimum_pages * 4,
        freelist_count: minimum_pages,
        page_size,
    })
    .unwrap());
    assert!(!should_reclaim(FreelistStats {
        page_count: minimum_pages * 4 + 1,
        freelist_count: minimum_pages,
        page_size,
    })
    .unwrap());
    assert!(!should_reclaim(FreelistStats {
        page_count: (minimum_pages - 1) * 4,
        freelist_count: minimum_pages - 1,
        page_size,
    })
    .unwrap());
    assert!(!should_reclaim(FreelistStats {
        page_count: 0,
        freelist_count: 0,
        page_size,
    })
    .unwrap());
}

#[test]
fn test_物理回収判定_整数演算のoverflowを発火扱いにしない() {
    assert!(matches!(
        should_reclaim(FreelistStats {
            page_count: u64::MAX,
            freelist_count: u64::MAX,
            page_size: u64::MAX,
        }),
        Err(MaintenanceFailure::ArithmeticOverflow)
    ));
}
