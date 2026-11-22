use serde_json::Value;

/// WebView requests only have access to the management operations the UI uses.
pub fn validate(method: &str, params: &Value) -> Result<(), String> {
    let fields: &[&str] = match method {
        "snapshot" | "changes.reconcile" | "gateway.pause" | "gateway.resume" => &[],
        "agents.launcher" => &["agent", "path"],
        "browser.open" => &["url"],
        "changes.apply" => &["planId"],
        "changes.history" => &["before"],
        "changes.preview" => &[
            "connectionIds",
            "targetSourceIds",
            "bindingTarget",
            "mode",
            "action",
            "replaceShared",
        ],
        "changes.preview_restore" | "changes.restore" => &["operationId"],
        "cli.check" | "cli.inspect" => &["manifest"],
        "cli.generate" => &["id", "name", "output", "tools"],
        "connections.adopt_source" | "connections.import" => &["sourceId", "name"],
        "connections.delete" => &["id"],
        "connections.detach" => &["id", "sourceId", "name"],
        "connections.detail"
        | "connections.reconnect"
        | "connections.tools"
        | "credentials.disconnect"
        | "gateway.refresh"
        | "oauth.start" => &["id"],
        "connections.export_config" => &["connectionIds", "targetAgent", "mode"],
        "connections.probe" => &["id", "expectedRevision"],
        "connections.save" => &[
            "id",
            "sourceId",
            "sourceName",
            "name",
            "title",
            "accountLabel",
            "definition",
            "credentialMappings",
        ],
        "connections.source_detail" => &["sourceId", "sourceName"],
        "credentials.update" => &["id", "header", "value"],
        "gateway.save" => &[
            "id",
            "editVersion",
            "title",
            "connectionIds",
            "toolSelections",
        ],
        "httpApi.import" => &["source", "serverUrl", "operations", "toolNames"],
        "oauth.configure" => &["id", "settings"],
        "sources.add" => &["agent", "scope", "path", "project"],
        "tools.call" => &["id", "tool", "schemaHash", "arguments"],
        "tools.describe" => &["id", "tool"],
        _ => return Err("此操作不开放给桌面界面".into()),
    };
    if params
        .as_object()
        .is_none_or(|params| params.keys().any(|key| !fields.contains(&key.as_str())))
    {
        return Err("桌面操作参数无效".into());
    }
    if method == "connections.probe" && params["expectedRevision"].as_u64().is_none() {
        return Err("请重新核对连接修订后确认探测".into());
    }
    Ok(())
}
