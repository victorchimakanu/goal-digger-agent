# Aomi Playground Example

This is the starter app behind Aomi's "deploy an example" onboarding. If you got
here from the Aomi portal, **you already have this agent deployed** — this repo
is its source. Clone it, change a tool, and redeploy to make it your own.

## What it does

Two tiny tools that show the shape of an Aomi app:

| Tool | Args | Returns |
|---|---|---|
| `echo` | `message: string` | the same message |
| `greet` | `name: string`, `excited?: bool` | a greeting |

## Make it yours

```bash
# 1. You created this repo from the template (the onboarding flow did this for you).
git clone https://github.com/<you>/playground-example
cd playground-example

# 2. Edit a tool, or add your own — copy the GreetTool pattern in src/lib.rs.
$EDITOR src/lib.rs

# 3. Compile-check locally.
cargo check

# 4. Redeploy. aomi-build sends a source-bound deploy to the Aomi backend, which
#    stages it, builds a release, and (after activation) loads your new version
#    into your chat session.
aomi-build deploy
```

Each redeploy from a new commit creates a fresh candidate build; activation
promotes it.

## Layout

```
playground-example/
├── aomi.toml      # app slug, display name, platform = "community", visibility
├── Cargo.toml     # cdylib + aomi-sdk pinned to the platform's required SDK version
├── src/lib.rs     # your tools + the dyn_aomi_app! registration
└── .gitignore
```

## Rules CI enforces

- `platform = "community"` in `aomi.toml`.
- `aomi-sdk` pinned EXACTLY (`= x.y.z`) to the community platform's
  `required_sdk_version` (see `platform.json` in `aomi-labs/community-apps`).
- `crate-type = ["cdylib"]`.

You do **not** hand-edit the platform repo — everything goes through
`aomi-build deploy`.
