"""Exercise the built renderer against an isolated CLI, using a browser in place of the Tauri webview."""
import argparse
import functools
import http.server
import json
from pathlib import Path
import secrets
import subprocess
import threading
import tomllib

from run import BINARY, Harness, ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--profile")
    parser.add_argument("--chrome", required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True)
    harness = Harness(record)
    secret = secrets.token_urlsafe(24)
    url = harness.url + "?tenant=sample&key=fixture"
    extra = harness.home / "second.toml"
    extra.write_text("# fixture source\n")
    result = {"conclusion":"FAIL"}
    checks = []

    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_POST(self):
            expected = f"127.0.0.1:{self.server.server_port}"
            if (self.path != "/rpc" or self.headers.get("Host") != expected
                    or self.headers.get("Origin") != "http://" + expected
                    or self.headers.get("X-Fixture") != secret):
                return self.send_error(403)
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if request.get("method") not in {"snapshot","connections.detail","connections.save","changes.preview","changes.apply","sources.add","connections.source_detail","connections.import","connections.detach","connections.probe","changes.preview_restore","changes.restore","credentials.disconnect","changes.history"}:
                return self.send_error(403)
            if request["method"] == "sources.add" and request.get("params") != {"agent":"codex","path":str(extra)}:
                return self.send_error(403)
            proc = subprocess.run([str(BINARY),"rpc","--stdin"], input=json.dumps(request),
                                  text=True, capture_output=True, timeout=30, env=harness.environment)
            self.send_response(200)
            self.send_header("Content-Type","application/json")
            self.end_headers()
            self.wfile.write(proc.stdout.encode())

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1",0), functools.partial(Handler,directory=str(ROOT/"dist")))
    origin = f"http://127.0.0.1:{server.server_port}"
    threading.Thread(target=server.serve_forever, daemon=True).start()
    shim = record / "fixture.js"
    shim.write_text("""
if (location.origin === ORIGIN) {
  let sequence = 0;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
  window.__TAURI_INTERNALS__ = {
    transformCallback() { return ++sequence; },
    async invoke(cmd, args) {
      if (cmd === "rpc") return (await fetch("/rpc", {
        method:"POST", headers:{"Content-Type":"application/json","X-Fixture":KEY},body:JSON.stringify(args)
      })).json();
      if (cmd.startsWith("plugin:event|")) return ++sequence;
      if (cmd === "plugin:dialog|open") return SOURCE_PATH;
      if (cmd === "desktop_status") return {supported:false};
      throw Error("Unexpected desktop operation");
    }
  };
}
""".replace("ORIGIN",json.dumps(origin)).replace("KEY",json.dumps(secret)).replace("SOURCE_PATH", json.dumps(str(extra))))

    def browser(*command):
        return subprocess.check_output(["agent-browser","--session",args.session,*command],
                                       text=True,timeout=55).strip()

    def audit(name):
        report = json.loads(browser("a11y","--json"))["data"]
        (record/f"a11y-{name}.json").write_text(json.dumps(report,indent=2))
        assert not report["violations"], [(v["id"],v["nodeCount"]) for v in report["violations"]]

    def wait_for(expression):
        browser("wait","--fn",expression)

    try:
        harness.rpc("connections.save", {"name":"public_http","title":"免认证服务","definition":{"url":url}})
        browser(*(["--profile",args.profile] if args.profile else []),
                "--executable-path",args.chrome,"--init-script",str(shim),"open",origin)
        browser("set","viewport","940","620")
        browser("wait","--text","免认证服务")
        audit("services")
        browser("find","role","button","click","--name","账号","--exact")
        wait_for("Array.from(document.querySelectorAll('article')).some(a=>a.textContent.includes('免认证服务') && a.textContent.includes('本机未配置独立凭据') && !a.textContent.includes('断开'))")
        checks.append("an unverified HTTP service shows the local credential state")
        browser("find","role","button","click","--name","Agent","--exact")
        browser("find","role","button","click","--name","添加来源","--exact")
        wait_for("document.querySelector('.agent-metrics strong')?.textContent === '2'")
        browser("find","first",".agent-services .cell-add","click")
        if "纳入托管并继续" in browser("snapshot"):
            browser("find","role","button","click","--name","纳入托管并继续")
        wait_for("document.querySelector('dialog[open]') && !document.querySelector('dialog input[type=checkbox]:checked')")
        browser("press","Escape")
        checks.append("Agent page adds a second Codex source and requires a target choice")
        browser("click","nav button:first-child")
        browser("find","role","button","click","--name","免认证服务 详情")
        browser("find","role","button","click","--name","编辑","--exact")
        browser("wait","--fn","document.querySelector('dialog input[required]') && !document.querySelector('dialog button.primary').disabled")
        browser("find","label","显示名称","fill","更名后的服务")
        browser("find","role","button","click","--name","保存连接")
        browser("wait","--text","更名后的服务")
        browser("click","article:first-of-type .service-card-footer > button")
        wait_for("document.querySelector('dialog[open]')?.contains(document.activeElement)")
        browser("press","Tab")
        wait_for("document.querySelector('dialog').contains(document.activeElement)")
        wait_for("document.querySelector('dialog footer').getBoundingClientRect().bottom <= innerHeight")
        audit("connect")
        browser("screenshot",str(record/"connect-min.png"))
        checks.append("minimum-size dialog keeps focus and preview button visible")
        browser("find","first","dialog label.target input","click")
        browser("find","role","button","click","--name","预览变更")
        browser("wait","--text","核对配置变更")
        browser("wait","--text","明文")
        browser("find","role","button","click","--name","应用配置")
        browser("wait","--text","接入结果")
        assert tomllib.loads((harness.home/".codex/config.toml").read_text())["mcp_servers"]["public_http"]["url"] == url
        checks.append("rename, direct-write disclosure, preview and apply preserve exact URL in native configuration")
        def button(name):
            browser("find","role","button","click","--name",name,"--exact")

        button("完成")
        readonly = harness.home / "review-readonly.json"
        readonly.write_text(json.dumps({"mcpServers":{"readonly_notes":{"url":harness.url}}}))
        harness.rpc("sources.add",{"agent":"claude","path":str(readonly)})
        shared = harness.rpc("connections.save",{"name":"managed_docs","title":"研发文档","definition":{
            "url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"}}})["id"]
        harness.rpc("connections.probe",{"id":shared})
        harness.changes(shared,["hermes:user"],mode="shared")
        unauth = harness.rpc("connections.save",{"name":"team_search","title":"团队检索","definition":{"url":harness.url}})["id"]
        harness.rpc("connections.probe",{"id":unauth})
        button("刷新本机配置")
        browser("wait","--text","readonly_notes")
        browser("set","viewport","1180","780")
        wait_for("[...document.querySelectorAll('article.service-card')].filter(a=>a.getBoundingClientRect().bottom<=innerHeight).length >= 5")
        browser("screenshot",str(record/"compact-list.png"))
        button("紧凑列表")
        wait_for("!document.querySelector('.service-list.compact')")
        button("紧凑列表")
        button("readonly_notes 详情")
        button("编辑并纳入托管")
        wait_for("!document.querySelector('dialog button.primary').disabled")
        button("取消")
        assert next(c for c in harness.rpc("snapshot")["connections"] if c["name"]=="readonly_notes")["detected"]
        button("readonly_notes 详情")
        button("编辑并纳入托管")
        wait_for("!document.querySelector('dialog button.primary').disabled")
        button("保存并纳入托管")
        wait_for("!document.querySelector('dialog[open]')")
        button("readonly_notes 详情")
        button("解除绑定")
        button("确认")
        wait_for("!document.querySelector('dialog[open]')")
        assert json.loads(readonly.read_text())["mcpServers"]["readonly_notes"]["url"]==harness.url
        assert any(c.get("detected") and c["name"]=="readonly_notes" for c in harness.rpc("snapshot")["connections"])
        checks.append("compact layout, explicit read-only adoption, cancel and detach preserving source")

        button("研发文档 详情")
        button("管理")
        wait_for("document.querySelector('dialog .target input:checked')?.closest('label').innerText.includes('Hermes') && document.querySelector('dialog input[name=mode]:checked')?.parentElement.innerText.includes('共享')")
        browser("screenshot",str(record/"manage-binding.png"))
        browser("press","Escape")
        button("研发文档 详情")
        button("编辑")
        wait_for("document.querySelector('dialog button.primary') && !document.querySelector('dialog button.primary').disabled")
        browser("find","label","MCP 地址","fill","http://127.0.0.1:1/mcp")
        wait_for("document.querySelector('dialog .warning-banner')?.innerText.includes('Hermes')")
        button("取消")
        button("研发文档 详情")
        button("检查工具")
        browser("wait","--text","search_documents")
        browser("press","Escape")
        button("账号")
        wait_for("[...document.querySelectorAll('article')].some(a=>a.innerText.includes('团队检索') && a.innerText.includes('需要认证') && a.innerText.includes('本机未配置独立凭据'))")
        button("断开")
        button("确认")
        wait_for("!document.querySelector('dialog[open]')")
        browser("click","nav button:first-child")
        button("研发文档 详情")
        wait_for("document.querySelector('dialog')?.innerText.includes('已断开') && !document.querySelector('dialog details.tool')")
        browser("press","Escape")
        checks.append("binding keeps Hermes/shared, address warning lists affected agents, disconnect removes stale tools")

        button("变更与恢复")
        browser("wait","--text","新增")
        browser("find","first",".history-row button","click")
        browser("wait","--text","核对恢复内容")
        browser("wait","--text","移除当前条目")
        browser("screenshot",str(record/"restore-preview.png"))
        button("返回记录")
        assert any(b.get("connectionId")==shared for b in harness.rpc("snapshot")["bindings"])
        browser("find","first",".history-row button","click")
        browser("wait","--text","核对恢复内容")
        button("确认恢复")
        wait_for("document.querySelector('dialog[aria-label=变更与恢复]') && !document.querySelector('dialog[aria-label=核对恢复内容]')")
        assert not any(b.get("connectionId")==shared for b in harness.rpc("snapshot")["bindings"])
        browser("press","Escape")
        checks.append("restore requires a preview, cancelling keeps the entry, confirming removes it")

        rotating = harness.rpc("connections.save",{"name":"rotating","title":"密钥轮换","definition":{
            "url":harness.url,"headers":{"Authorization":"synthetic-before"}}})["id"]
        harness.changes(rotating,["codex:user"],mode="direct")
        harness.rpc("credentials.update",{"id":rotating,"value":"synthetic-after"})
        button("刷新本机配置")
        browser("wait","--text","密钥轮换")
        button("密钥轮换 详情")
        button("管理")
        button("预览变更")
        wait_for("document.querySelector('.field-changes')?.innerText.includes('Authorization · 已更换')")
        assert "synthetic-before" not in browser("snapshot") and "synthetic-after" not in browser("snapshot")
        browser("screenshot",str(record/"credential-diff.png"))
        checks.append("secret rotation is visible in the field diff without either credential value")
        result["conclusion"]="PASS"
    finally:
        try:
            browser("close")
        finally:
            server.shutdown()
            server.server_close()
            harness.close(False)
            shim.unlink(missing_ok=True)
            (record/"report.json").write_text(json.dumps({**result,"checks":checks},indent=2)+"\n")
    print("PASS", record)


if __name__ == "__main__":
    main()
