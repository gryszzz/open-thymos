//! Tests for the manifest-driven tool loader.

use serde_json::json;
use std::io::Write;
use thymos_core::world::World;
use thymos_tools::{
    KvGetTool, ManifestTool, ToolContract, ToolInvocation, ToolManifest, ToolRegistry,
};

fn sample_manifest() -> serde_json::Value {
    json!({
        "name": "echo_greeting",
        "version": "1.0.0",
        "description": "Echo a greeting message via shell",
        "effect_class": "external",
        "risk_class": "low",
        "input_schema": {
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        },
        "executor": {
            "kind": "shell",
            "command_template": "echo hello {name}"
        }
    })
}

#[test]
fn manifest_from_json_roundtrip() {
    let manifest: ToolManifest = serde_json::from_value(sample_manifest()).unwrap();
    assert_eq!(manifest.name, "echo_greeting");
    assert_eq!(manifest.version, "1.0.0");

    let tool = ManifestTool::from_manifest(manifest);
    assert_eq!(tool.meta().name, "echo_greeting");
    assert_eq!(tool.description(), "Echo a greeting message via shell");
    assert!(tool.input_schema()["properties"]["name"].is_object());
}

#[test]
fn manifest_rejects_unsafe_tool_name() {
    let manifest: ToolManifest = serde_json::from_value(json!({
        "name": "../fs_patch",
        "version": "1.0.0",
        "description": "Bad manifest name",
        "effect_class": "pure",
        "risk_class": "low",
        "input_schema": { "type": "object" },
        "executor": { "kind": "noop" }
    }))
    .unwrap();

    let err = match ManifestTool::try_from_manifest(manifest) {
        Ok(_) => panic!("unsafe manifest name should be rejected"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("must contain only ASCII"),
        "unexpected error: {err}"
    );
}

#[test]
fn manifest_noop_executor() {
    let manifest: ToolManifest = serde_json::from_value(json!({
        "name": "dry_run",
        "version": "0.1.0",
        "description": "A no-op test tool",
        "effect_class": "pure",
        "risk_class": "low",
        "input_schema": { "type": "object" },
        "executor": { "kind": "noop" }
    }))
    .unwrap();

    let tool = ManifestTool::from_manifest(manifest);
    let world = World::default();
    let args = json!({});
    let outcome = tool
        .execute(&ToolInvocation {
            args: &args,
            world: &world,
        })
        .unwrap();

    assert_eq!(outcome.observation.output["result"], "noop");
}

#[test]
fn manifest_shell_executor() {
    let manifest: ToolManifest = serde_json::from_value(sample_manifest()).unwrap();
    let tool = ManifestTool::from_manifest(manifest);
    let world = World::default();
    let args = json!({ "name": "thymos" });

    let outcome = tool
        .execute(&ToolInvocation {
            args: &args,
            world: &world,
        })
        .unwrap();

    let stdout = outcome.observation.output["stdout"].as_str().unwrap();
    assert!(stdout.contains("hello"), "expected 'hello' in: {stdout}");
    assert!(stdout.contains("thymos"), "expected 'thymos' in: {stdout}");
}

#[test]
fn load_manifest_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("my_tool.json");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(
        serde_json::to_string_pretty(&sample_manifest())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();

    let mut registry = ToolRegistry::new();
    registry.load_manifest(&path).unwrap();
    assert!(registry.get("echo_greeting").is_ok());
}

#[test]
fn manifest_cannot_shadow_existing_tool() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kv_get.json");
    let manifest = json!({
        "name": "kv_get",
        "version": "9.9.9",
        "description": "Attempt to replace built-in kv_get",
        "effect_class": "pure",
        "risk_class": "low",
        "input_schema": { "type": "object" },
        "executor": { "kind": "noop" }
    });
    std::fs::write(&path, serde_json::to_string(&manifest).unwrap()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(KvGetTool::default());

    let err = registry.load_manifest(&path).unwrap_err();
    assert!(
        err.to_string()
            .contains("conflicts with an existing registered tool"),
        "unexpected error: {err}"
    );
    assert_eq!(registry.get("kv_get").unwrap().meta().version, "0.0.1");
}

#[test]
fn load_manifest_dir() {
    let dir = tempfile::tempdir().unwrap();

    // Write two valid manifests and one non-json file.
    for (name, tool_name) in [("a.json", "tool_a"), ("b.json", "tool_b")] {
        let manifest = json!({
            "name": tool_name,
            "version": "0.1.0",
            "description": format!("Test tool {tool_name}"),
            "effect_class": "pure",
            "risk_class": "low",
            "input_schema": { "type": "object" }
        });
        std::fs::write(
            dir.path().join(name),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }
    std::fs::write(dir.path().join("readme.txt"), "not a manifest").unwrap();

    let mut registry = ToolRegistry::new();
    let count = registry.load_manifest_dir(dir.path()).unwrap();
    assert_eq!(count, 2);
    assert!(registry.get("tool_a").is_ok());
    assert!(registry.get("tool_b").is_ok());
}

#[test]
fn shell_escape_prevents_injection() {
    let manifest: ToolManifest = serde_json::from_value(json!({
        "name": "safe_echo",
        "version": "0.1.0",
        "description": "Test shell escaping",
        "effect_class": "external",
        "risk_class": "low",
        "input_schema": { "type": "object", "properties": { "msg": { "type": "string" } } },
        "executor": { "kind": "shell", "command_template": "echo {msg}" }
    }))
    .unwrap();

    let tool = ManifestTool::from_manifest(manifest);
    let world = World::default();
    // Try to inject a semicolon command.
    let args = json!({ "msg": "hello; rm -rf /" });
    let outcome = tool
        .execute(&ToolInvocation {
            args: &args,
            world: &world,
        })
        .unwrap();

    let stdout = outcome.observation.output["stdout"].as_str().unwrap();
    // The semicolon should be treated as literal text, not a command separator.
    assert!(
        stdout.contains("rm -rf"),
        "the injected text should appear as literal output, got: {stdout}"
    );
}
