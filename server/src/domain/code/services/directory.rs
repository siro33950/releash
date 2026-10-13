pub fn aggregate_status<'a>(statuses: impl Iterator<Item = &'a str>) -> &'static str {
    let mut priority = 0;
    for status in statuses {
        priority = priority.max(match status {
            "conflicted" => 4,
            "deleted" => 3,
            "modified" | "renamed" => 2,
            "new" => 1,
            _ => 0,
        });
    }
    match priority {
        4 => "conflicted",
        3 => "deleted",
        2 => "modified",
        1 => "new",
        _ => "none",
    }
}

#[cfg(test)]
#[path = "directory_test.rs"]
mod directory_tests;
