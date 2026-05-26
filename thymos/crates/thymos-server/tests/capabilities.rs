use serde_json::json;

#[test]
fn default_runtime_loads_manifest_capabilities() {
    let unique = format!(
        "openthymos-capabilities-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();

    let manifest = json!({
        "name": "capability_echo",
        "version": "0.1.0",
        "description": "Echo test capability",
        "effect_class": "pure",
        "risk_class": "low",
        "input_schema": {
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        },
        "executor": { "kind": "noop" }
    });
    std::fs::write(
        dir.join("capability_echo.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let dirs = vec![dir.display().to_string()];
    let runtime = thymos_server::default_runtime_with_capabilities(&dirs);

    assert!(runtime.tools.get("capability_echo").is_ok());

    let _ = std::fs::remove_dir_all(dir);
}
