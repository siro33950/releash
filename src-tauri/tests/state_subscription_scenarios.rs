include!("../src/lib.rs");

use crate::usecase::state_subscription::*;
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use futures_util::{Stream, StreamExt};
use parking_lot::Mutex;
use std::sync::Arc;

#[path = "state_subscription_scenarios/scenarios_test.rs"]
mod state_subscription_scenarios_tests;
#[path = "state_subscription_terminal/terminal_test.rs"]
mod terminal_scenarios_tests;
