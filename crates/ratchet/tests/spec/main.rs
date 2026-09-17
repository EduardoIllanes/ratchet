// Scenario test names are `<spec>__<slug>` by convention (checked by tests/scenarios.rs);
// the double underscore trips the snake_case lint, so it is disabled crate-wide here.
#![allow(non_snake_case)]

mod agent_protocol;
mod board;
mod pdf;
mod sessions;
mod support;
mod tasks;
