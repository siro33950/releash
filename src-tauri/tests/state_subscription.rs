#[cfg(all(debug_assertions, feature = "desktop"))]
#[path = "state_subscription/flow_test.rs"]
mod flow_test;

#[cfg(debug_assertions)]
#[path = "state_subscription/terminal_test.rs"]
mod terminal_test;
