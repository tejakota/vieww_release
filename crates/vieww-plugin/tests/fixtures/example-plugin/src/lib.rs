//! A real, minimal `vieww` plugin — the fixture
//! `vieww-plugin/tests/example_plugin.rs` compiles into a genuine `cdylib`
//! and `dlopen`s, so that crate's end-to-end test exercises the actual ABI
//! boundary rather than only the in-process trampolines `abi.rs`'s own unit
//! tests already cover.

use std::cell::Cell;

use vieww_plugin::{vieww_plugin, Plugin, PluginCommand};

/// Counts how many times `greet` has run, purely so a test can observe that
/// state survives across repeated `invoke` calls made through the real ABI.
#[vieww_plugin]
pub struct ExamplePlugin {
    greet_count: Cell<u32>,
}

impl Default for ExamplePlugin {
    fn default() -> Self {
        Self { greet_count: Cell::new(0) }
    }
}

impl Plugin for ExamplePlugin {
    fn name(&self) -> &str {
        "vieww-example-plugin"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn commands(&self) -> Vec<PluginCommand> {
        vec![
            PluginCommand::new("greet", "Say hello"),
            PluginCommand::new("fail", "Always fails, to exercise the error path across the real ABI"),
        ]
    }

    fn invoke(&mut self, id: &str) -> Result<(), String> {
        match id {
            "greet" => {
                self.greet_count.set(self.greet_count.get() + 1);
                Ok(())
            }
            "fail" => Err("this command always fails, on purpose".to_owned()),
            // The trampoline in `vieww-plugin::abi` checks every id against
            // the commands snapshot before this is ever reached — see
            // `Plugin::invoke`'s own doc.
            _ => unreachable!("the ABI trampoline rejects unknown ids before this is reached"),
        }
    }
}
