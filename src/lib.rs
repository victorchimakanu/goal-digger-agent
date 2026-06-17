//! Aomi Playground — a minimal starter app you can clone, edit, and redeploy.
//!
//! This is the source for the example agent you deployed during onboarding.
//! Each tool is a zero-sized type implementing `DynAomiTool`; the
//! `dyn_aomi_app!` macro at the bottom registers them and exposes the plugin to
//! the Aomi backend. To make it yours: edit a tool, add a new one, then run
//! `aomi-build deploy`.

use aomi_sdk::{DynAomiTool, DynToolCallCtx, dyn_aomi_app};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// App state. Keep it `Clone + Default` — the runtime constructs one per
/// session. Add fields here (HTTP clients, config) as your app grows.
#[derive(Clone, Default)]
struct PlaygroundApp;

// ---------------------------------------------------------------------------
// Tool 1 — echo: the simplest possible tool (one required string arg).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct EchoArgs {
    /// The message to echo back.
    message: String,
}

struct EchoTool;

impl DynAomiTool for EchoTool {
    type App = PlaygroundApp;
    type Args = EchoArgs;

    const NAME: &'static str = "echo";
    const DESCRIPTION: &'static str = "Echo a message back verbatim.";

    fn run(_app: &Self::App, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        Ok(json!({ "message": args.message }))
    }
}

// ---------------------------------------------------------------------------
// Tool 2 — greet: shows optional args + returning structured JSON. Copy this
// shape to build your own tools.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct GreetArgs {
    /// Who to greet.
    name: String,
    /// Add an exclamation mark. Defaults to false.
    #[serde(default)]
    excited: bool,
}

struct GreetTool;

impl DynAomiTool for GreetTool {
    type App = PlaygroundApp;
    type Args = GreetArgs;

    const NAME: &'static str = "greet";
    const DESCRIPTION: &'static str = "Greet someone by name, optionally with excitement.";

    fn run(_app: &Self::App, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let punct = if args.excited { "!" } else { "." };
        Ok(json!({ "greeting": format!("Hello, {}{}", args.name, punct) }))
    }
}

dyn_aomi_app!(
    app = PlaygroundApp,
    name = "playground-example",
    version = "0.1.0",
    preamble = "You are the Aomi Playground example agent. You can echo messages \
                and greet people. Encourage the user to clone this repo, edit \
                `src/lib.rs`, and redeploy with `aomi-build` to make it their own.",
    tools = [EchoTool, GreetTool],
    namespaces = []
);
